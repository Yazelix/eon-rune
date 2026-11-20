//! The Wayland data-control clipboard: what each write offers, `text/uri-list` framing, capped
//! reads that leave the offer alive, and the error mapping. The `live_` tests need a compositor with
//! data-control started for them: they run only when asked (`--ignored`), one at a time, with
//! `RUNE_CLIPBOARD_LIVE_TEST=1`, since they write and clear the clipboard.
#![cfg(target_os = "linux")]

use super::wayland::{Capped, capped, drain, read_outcome, sources, uri_list, uri_list_paths};
use super::*;
use std::io::{Read, Write};
use std::path::PathBuf;
use wl_clipboard_rs::{copy, paste};

fn offered(write: &ClipboardWrite) -> Vec<(String, Vec<u8>)> {
    sources(write).unwrap().into_iter().map(|s| {
        let mime = match s.mime_type { copy::MimeType::Text => "<text>".to_owned(), copy::MimeType::Specific(m) => m, copy::MimeType::Autodetect => "<auto>".to_owned() };
        let copy::Source::Bytes(bytes) = s.source else { panic!("only bytes are offered") };
        (mime, bytes.into_vec())
    }).collect()
}

fn pair(mime: &str, bytes: &[u8]) -> (String, Vec<u8>) { (mime.to_owned(), bytes.to_vec()) }

fn png() -> ClipboardImage { ClipboardImage::from_rgba(1, 1, &[255, 0, 0, 255]).unwrap() }

#[test]
fn each_write_offers_the_forms_arboard_offers() {
    let text = ClipboardWrite::new(ClipboardContent::new("héllo"));
    assert_eq!(offered(&text), [pair("<text>", "héllo".as_bytes())], "text: the whole text family");

    let html = ClipboardWrite::new(ClipboardContent::new("alt").with_html("<b>x</b>"));
    assert_eq!(offered(&html), [pair("<text>", b"alt"), pair("text/html", b"<b>x</b>")]);

    let image = png();
    let picture = ClipboardWrite::new(ClipboardContent::new("ignored").with_image(image.clone()));
    assert_eq!(offered(&picture), [pair("image/png", image.png())], "the image alone, as Rune's own PNG");

    let file = std::env::current_exe().unwrap();
    let files = ClipboardWrite::new(ClipboardContent::new("ignored").with_image(image).with_files([file.clone()]));
    let canonical = file.canonicalize().unwrap();
    assert_eq!(offered(&files), [pair("text/uri-list", crate::file_uri::uri(&canonical).as_bytes())], "files win over the image");

    let (secret, _copy) = ClipboardWrite::secret("pw".into(), Duration::from_secs(60));
    assert_eq!(offered(&secret), [pair("<text>", b"pw"), pair("x-kde-passwordManagerHint", b"secret")]);
}

#[test]
fn a_secret_is_its_text_even_when_written_rich() {
    let (secret, _copy) = ClipboardWrite::secret("pw".into(), Duration::from_secs(60));
    let types: Vec<_> = offered(&secret).into_iter().map(|(m, _)| m).collect();
    assert_eq!(types, ["<text>", "x-kde-passwordManagerHint"]);
}

#[test]
fn copying_only_missing_files_is_an_error() {
    let gone = std::env::temp_dir().join("rune-wayland-tests-no-such-file");
    assert!(uri_list(&[gone]).is_err());
}

#[test]
fn uri_lists_round_trip_and_follow_rfc_2483_framing() {
    let dir = std::env::temp_dir().join(format!("rune wayland é%{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("a b.txt"), dir.join("100%.txt"));
    std::fs::write(&a, b"").unwrap();
    std::fs::write(&b, b"").unwrap();
    let list = uri_list(&[a.clone(), b.clone()]).unwrap();
    assert_eq!(list.matches("\r\n").count(), 1, "CRLF between URIs, none after the last");
    assert!(!list.contains(' ') && !list.contains('é'), "spaces and bytes past ASCII are encoded: {list}");
    assert_eq!(uri_list_paths(list.as_bytes()), [a.canonicalize().unwrap(), b.canonicalize().unwrap()]);
    std::fs::remove_dir_all(&dir).unwrap();

    let framed = b"# a comment\r\nfile:///tmp/a%20b\r\n\r\nhttps://example.com/x\nfile://localhost/tmp/c\nfile://remote/tmp/d\n";
    assert_eq!(uri_list_paths(framed), [PathBuf::from("/tmp/a b"), PathBuf::from("/tmp/c")], "comments, blank lines and non-local URIs skipped; bare LF accepted");
}

/// Hands out `left` bytes and counts what was taken.
struct Counting { left: usize, taken: usize }
impl Read for Counting {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.left);
        buf[..n].fill(b'a');
        self.left -= n;
        self.taken += n;
        Ok(n)
    }
}

#[test]
fn a_capped_read_takes_at_most_one_byte_past_its_cap() {
    let mut big = Counting { left: 10_000, taken: 0 };
    assert_eq!(capped(&mut big, 100), Some(Capped::Over));
    assert_eq!(big.taken, 101);
    let mut small = Counting { left: 100, taken: 0 };
    assert_eq!(capped(&mut small, 100), Some(Capped::Fits(vec![b'a'; 100])));
}

/// A writer that finishes is drained to its end: it completes, rather than blocking on a full
/// pipe or meeting a broken one. The drain limit and the wait are watchdogs, not the rule.
#[test]
fn a_refused_read_drains_so_the_writer_finishes() {
    let (mut reader, mut writer) = std::io::pipe().unwrap();
    let (wrote, written) = mpsc::channel();
    std::thread::spawn(move || wrote.send(writer.write_all(&vec![b'x'; 1 << 20]).is_ok()).unwrap());
    assert_eq!(capped(&mut reader, 1024), Some(Capped::Over));
    drain(&mut reader, Duration::from_secs(60));
    drop(reader);
    assert_eq!(written.recv_timeout(Duration::from_secs(30)), Ok(true), "the writer saw every byte taken, not a broken pipe");
}

#[test]
fn draining_a_writer_that_never_ends_stops_at_the_limit() {
    let (mut reader, writer) = std::io::pipe().unwrap();
    let start = Instant::now();
    drain(&mut reader, Duration::from_millis(200));
    let took = start.elapsed();
    // Load can only lengthen the wait, so the lower bound is the rule and the upper a watchdog.
    assert!(took >= Duration::from_millis(200) && took < Duration::from_secs(30), "{took:?}");
    drop(writer);
}

#[test]
fn read_errors_map_to_outcomes() {
    use paste::Error;
    use plain::Outcome;
    assert_eq!(read_outcome(&Error::ClipboardEmpty), Outcome::Empty);
    assert_eq!(read_outcome(&Error::NoMimeType), Outcome::NonText);
    assert_eq!(read_outcome(&Error::MissingProtocol { name: "zwlr_data_control_manager_v1", version: 1 }), Outcome::Unsupported);
    assert_eq!(read_outcome(&Error::PrimarySelectionUnsupported), Outcome::Unsupported);
    assert_eq!(read_outcome(&Error::WaylandConnection(wayland_client::ConnectError::NoCompositor)), Outcome::Unsupported);
    assert_eq!(read_outcome(&Error::NoSeats), Outcome::HostFailure);
}

#[cfg(not(rune_x11))]
mod live {
    use super::*;
    use super::super::native::Worker;
    use super::super::wayland::Wayland;

    fn types() -> Vec<String> {
        let mut t: Vec<_> = paste::get_mime_types(paste::ClipboardType::Regular, paste::Seat::Unspecified).unwrap().into_iter().collect();
        t.sort();
        t
    }
    fn read(mime: &str) -> Result<Vec<u8>, paste::Error> {
        let (mut pipe, _) = paste::get_contents(paste::ClipboardType::Regular, paste::Seat::Unspecified, paste::MimeType::Specific(mime))?;
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).unwrap();
        Ok(bytes)
    }
    fn empty() -> bool { matches!(read("text/plain"), Err(paste::Error::ClipboardEmpty)) }
    fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(30);
        while !done() {
            assert!(Instant::now() < until, "{what} within 30 s");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    /// The live tests write and clear the session clipboard, so they run only where a harness
    /// started a compositor for them and says so (P4's private Sway).
    #[allow(clippy::disallowed_methods, reason = "live test checks its isolation guard")]
    fn isolated() {
        assert_eq!(std::env::var("RUNE_CLIPBOARD_LIVE_TEST").as_deref(), Ok("1"), "set by a harness that started its own compositor");
    }
    fn sync(worker: &Worker) {
        let (reply, received) = mpsc::channel();
        worker.sender.send(Request::Read { purpose: Purpose::Paste(None, Forms::Text), reply, wake: Arc::new(|| {}) }).unwrap();
        received.recv_timeout(Duration::from_secs(30)).unwrap();
    }

    /// Every form through the backend, read back in this process and as another client would.
    #[test]
    #[ignore = "needs a Wayland compositor with data-control (WAYLAND_DISPLAY)"]
    fn live_wayland_round_trip() {
        isolated();
        let mut cb = Wayland;
        cb.write(&ClipboardWrite::new(ClipboardContent::new("héllo"))).unwrap();
        for mime in ["UTF8_STRING", "text/plain", "text/plain;charset=utf-8"] {
            assert_eq!(read(mime).unwrap(), "héllo".as_bytes(), "{mime}");
        }
        assert_eq!(cb.text().as_deref().map(String::as_str), Some("héllo"));
        assert_eq!(cb.bounded_text(false, 1024), plain::Outcome::Text("héllo".into()));

        cb.write(&ClipboardWrite::new(ClipboardContent::new("alt").with_html("<b>x</b>"))).unwrap();
        assert!(types().contains(&"text/html".to_owned()));
        let content = cb.content(false).unwrap();
        assert_eq!((content.text(), content.html()), ("alt", Some("<b>x</b>")));

        let image = png();
        cb.write(&ClipboardWrite::new(ClipboardContent::new("").with_image(image.clone()))).unwrap();
        assert_eq!(types(), ["image/png"]);
        assert_eq!(cb.content(true).unwrap().image().map(ClipboardImage::png), Some(image.png()));

        let file = std::env::current_exe().unwrap().canonicalize().unwrap();
        cb.write(&ClipboardWrite::new(ClipboardContent::new("").with_files([file.clone()]))).unwrap();
        // wl-clipboard-rs counts text/uri-list as text and adds the text family, as it does for
        // arboard's own file-list write.
        assert_eq!(types(), ["STRING", "TEXT", "UTF8_STRING", "text/plain", "text/plain;charset=utf-8", "text/uri-list"]);
        assert_eq!(cb.content(true).unwrap().files(), [file]);

        let (secret, _copy) = ClipboardWrite::secret("pw".into(), Duration::from_secs(60));
        cb.write(&secret).unwrap();
        assert_eq!(read("x-kde-passwordManagerHint").unwrap(), b"secret");
        assert_eq!(read("UTF8_STRING").unwrap(), b"pw");

        cb.write_primary("primary".into()).unwrap();
        assert_eq!(cb.primary_text().as_deref(), Some("primary"));
        cb.clear_primary().unwrap();
        eventually("the primary offer gone", || cb.primary_text().is_none());

        cb.clear().unwrap();
        eventually("the offer gone after clear", empty);

        // A rich read refused for size leaves the offer alive for everyone.
        let huge = "a".repeat(html::MAX_RICH_BYTES + 1);
        cb.write(&ClipboardWrite::new(ClipboardContent::new("alt").with_html(huge))).unwrap();
        assert_eq!(cb.content(false).unwrap().html(), None, "over the cap: refused");
        assert_eq!(read("text/plain").unwrap(), b"alt", "the offer survived the refused read");
        cb.clear().unwrap();
    }

    /// A secret leaves the clipboard when its hold expires, and when the worker stops.
    #[test]
    #[ignore = "needs a Wayland compositor with data-control (WAYLAND_DISPLAY)"]
    fn live_wayland_secret_expiry_and_shutdown() {
        isolated();
        let worker = Worker::spawn(|| Some(Wayland));
        worker.sender.send(Request::Write(ClipboardWrite::secret("expiring".into(), Duration::from_secs(2)).0, None)).unwrap();
        sync(&worker);
        assert_eq!(read("text/plain").unwrap(), b"expiring", "the secret is offered before its hold ends");
        eventually("the expired secret gone", empty);

        worker.sender.send(Request::Write(ClipboardWrite::secret("held".into(), Duration::from_secs(3600)).0, None)).unwrap();
        sync(&worker);
        assert_eq!(read("text/plain").unwrap(), b"held");
        assert!(worker.stop(), "the worker acknowledged its cleanup");
        eventually("the held secret gone after stop", empty);
    }

    /// No compositor: reads are unsupported and writes fail, nothing hangs. Run with
    /// `WAYLAND_DISPLAY` naming a socket that doesn't exist.
    #[test]
    #[ignore = "run with WAYLAND_DISPLAY naming a missing socket"]
    fn live_wayland_missing_compositor() {
        isolated();
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_default();
        let display = std::env::var_os("WAYLAND_DISPLAY").map(PathBuf::from).unwrap_or_default();
        assert!(!display.as_os_str().is_empty() && !runtime.join(&display).exists(), "WAYLAND_DISPLAY names a socket that doesn't exist");
        let mut cb = Wayland;
        assert_eq!(cb.bounded_text(false, 1024), plain::Outcome::Unsupported);
        assert!(cb.write(&ClipboardWrite::new(ClipboardContent::new("x"))).is_err());
        assert!(cb.write_primary("x".into()).is_err());
        assert!(cb.text().is_none() && cb.content(true).is_none());
    }
}
