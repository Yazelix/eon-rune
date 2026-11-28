//! The Wayland data-control clipboard: what each write offers, `text/uri-list` framing, capped
//! reads that leave the offer alive, and the error mapping. The `live_` tests need a compositor with
//! data-control started for them: they run only when asked (`--ignored`), one at a time, with
//! `RUNE_CLIPBOARD_LIVE_TEST=1`, since they write and clear the clipboard.
#![cfg(target_os = "linux")]

use super::wayland::{Capped, Next, Within, capped, drain, next, open, read_outcome, sources, uri_list, uri_list_paths};
use super::*;
use std::io::{ErrorKind, Read, Write};
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
    assert_eq!(offered(&files), [pair("text/uri-list", uri_list(&[canonical]).unwrap().as_bytes())], "files win over the image");

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

    let framed = b"# a comment\r\nfile:///tmp/a%20b\r\n\r\nhttps://example.com/x\nfile://localhost/tmp/c\nfile://remote/tmp/d\nfile:///tmp/a%GG\nfile:///tmp/nul%00\n";
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
/// pipe or meeting a broken one. The deadline and the wait are watchdogs, not the rule.
#[test]
fn a_refused_read_drains_so_the_writer_finishes() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let mut reader = Within { pipe: reader, until: crate::time::now() + Duration::from_secs(60) };
    let (wrote, written) = mpsc::channel();
    std::thread::spawn(move || wrote.send(writer.write_all(&vec![b'x'; 1 << 20]).is_ok()).unwrap());
    assert_eq!(capped(&mut reader, 1024), Some(Capped::Over));
    drain(&mut reader);
    drop(reader);
    assert_eq!(written.recv_timeout(Duration::from_secs(30)), Ok(true), "the writer saw every byte taken, not a broken pipe");
}

/// The deadline decision: a read happens only while time is left, so a `poll` that wakes at or
/// past `until` (ready or not) times out.
#[test]
fn a_deadline_read_never_reads_at_or_past_until() {
    let until = crate::time::now();
    let ms = Duration::from_millis(1);
    for (now, ready, want) in [
        (until - ms, true, Next::Read),
        (until - ms, false, Next::Wait),
        (until, true, Next::TimedOut),
        (until, false, Next::TimedOut),
        (until + ms, true, Next::TimedOut),
        (until + ms, false, Next::TimedOut),
    ] {
        assert_eq!(next(now, until, ready), want, "{:?} past the deadline, ready {ready}", now.saturating_duration_since(until));
    }
}

/// A pipe read by a deadline takes what a provider wrote while time is left and fails once it has
/// passed, even with bytes waiting. Only the empty-pipe row waits, for its 10 ms `poll`; the
/// other rows' bytes are already in the pipe, and the far deadline is a watchdog.
#[test]
fn a_pipe_read_ends_at_its_deadline() {
    let far = || crate::time::now() + Duration::from_secs(60);
    let piped = |bytes: &[u8]| {
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(bytes).unwrap();
        (reader, writer)
    };
    let timed_out = |r: std::io::Result<usize>| r.map_err(|e| e.kind()) == Err(ErrorKind::TimedOut);
    let mut buf = [0; 16];
    // The deadline passes mid-transfer: the prefix was read, the rest is refused.
    let (pipe, mut writer) = piped(b"prefix");
    let mut reading = Within { pipe, until: far() };
    assert_eq!(reading.read(&mut buf).unwrap(), 6);
    writer.write_all(b"more").unwrap();
    reading.until = crate::time::now();
    assert!(timed_out(reading.read(&mut buf)));
    // A whole text and its end arrive in time.
    let (pipe, _) = piped(b"whole");
    assert_eq!(plain::decode(Within { pipe, until: far() }, 64), plain::Outcome::Text("whole".into()));
    // Over the cap with the writer open: refused without waiting for the end.
    let (pipe, _writer) = piped(b"123456789");
    assert_eq!(plain::decode(Within { pipe, until: far() }, 8), plain::Outcome::TooLarge);
    // A refused rich read drains on the same deadline: not at all once it has passed.
    let (pipe, _) = piped(b"123456789abc");
    let mut rich = Within { pipe, until: far() };
    assert_eq!(capped(&mut rich, 8), Some(Capped::Over));
    rich.until = crate::time::now();
    drain(&mut rich);
    assert_eq!(rich.pipe.read(&mut buf).unwrap(), 3, "the unread suffix stays in the pipe");
    // An open pipe that never gets a byte: `poll` times out and the read fails at the deadline.
    let (pipe, writer) = std::io::pipe().unwrap();
    let (sent, got) = mpsc::channel();
    std::thread::spawn(move || sent.send(Within { pipe, until: crate::time::now() + Duration::from_millis(10) }.read(&mut [0; 16]).map_err(|e| e.kind())).ok());
    assert_eq!(got.recv_timeout(Duration::from_secs(30)), Ok(Err(ErrorKind::TimedOut)));
    drop(writer);
    // Once the deadline has passed no pipe is opened, so no compositor is asked.
    assert_eq!(open(paste::ClipboardType::Regular, paste::MimeType::Text, crate::time::now()).err(), Some(plain::Outcome::HostFailure));
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
        let until = crate::time::now() + Duration::from_secs(30);
        while !done() {
            assert!(crate::time::now() < until, "{what} within 30 s");
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

    /// A private data-control source that answers every `send` with a prefix and keeps the pipe
    /// open: a provider that stalls mid-transfer. It reports each `send` (with its type) and its
    /// cancellation in order on one channel.
    mod stalled {
        use std::{fs::File, io::Write, sync::mpsc};
        use wayland_client::globals::{GlobalListContents, registry_queue_init};
        use wayland_client::protocol::{wl_registry::WlRegistry, wl_seat::WlSeat};
        use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop, event_created_child};
        use wayland_protocols_wlr::data_control::v1::client::{zwlr_data_control_device_v1::{self as device, ZwlrDataControlDeviceV1}, zwlr_data_control_manager_v1::ZwlrDataControlManagerV1};
        use wayland_protocols_wlr::data_control::v1::client::{zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, zwlr_data_control_source_v1::{Event, ZwlrDataControlSourceV1}};

        #[derive(Debug, PartialEq)]
        pub enum Seen { Sent(String), Cancelled }
        struct State { seen: mpsc::Sender<Seen>, _held: Vec<File>, done: bool }
        impl Dispatch<WlRegistry, GlobalListContents> for State {
            fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as wayland_client::Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
        }
        delegate_noop!(State: ignore WlSeat);
        delegate_noop!(State: ZwlrDataControlManagerV1);
        delegate_noop!(State: ignore ZwlrDataControlOfferV1);
        impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
            fn event(_: &mut Self, _: &ZwlrDataControlDeviceV1, _: device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
            event_created_child!(State, ZwlrDataControlDeviceV1, [device::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ())]);
        }
        impl Dispatch<ZwlrDataControlSourceV1, ()> for State {
            fn event(state: &mut Self, _: &ZwlrDataControlSourceV1, event: Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
                match event {
                    // Sends a prefix and holds the pipe open; the reader may already be gone.
                    Event::Send { mime_type, fd } => {
                        let mut pipe = File::from(fd);
                        pipe.write_all(b"partial").ok();
                        state._held.push(pipe);
                        state.seen.send(Seen::Sent(mime_type)).ok();
                    }
                    Event::Cancelled => { state.done = true; state.seen.send(Seen::Cancelled).ok(); }
                    _ => {}
                }
            }
        }

        /// Offers text and HTML on the regular selection from its own connection, returning once
        /// the compositor holds the offer. The second receiver hears when the source thread ends,
        /// which drops every pipe it held.
        pub fn start() -> (mpsc::Receiver<Seen>, mpsc::Receiver<()>) {
            let (seen, saw) = mpsc::channel();
            let (ready, readied) = mpsc::channel();
            let (exited, ended) = mpsc::channel();
            std::thread::spawn(move || {
                let conn = Connection::connect_to_env().unwrap();
                let (globals, mut queue) = registry_queue_init::<State>(&conn).unwrap();
                let qh = queue.handle();
                let seat: WlSeat = globals.bind(&qh, 1..=1, ()).unwrap();
                let manager: ZwlrDataControlManagerV1 = globals.bind(&qh, 1..=2, ()).unwrap();
                let device = manager.get_data_device(&seat, &qh, ());
                let source = manager.create_data_source(&qh, ());
                source.offer("text/plain;charset=utf-8".into());
                source.offer("text/html".into());
                device.set_selection(Some(&source));
                let mut state = State { seen, _held: Vec::new(), done: false };
                queue.roundtrip(&mut state).unwrap();
                ready.send(()).unwrap();
                while !state.done && queue.blocking_dispatch(&mut state).is_ok() {}
                drop(state);
                exited.send(()).ok();
            });
            readied.recv_timeout(super::Duration::from_secs(30)).expect("the stalled source owns the selection");
            (saw, ended)
        }
    }

    /// Ends the stalled source however the test ends: clearing the selection cancels it, and its
    /// thread then drops the pipes it held. Fails the test if either clear fails or hangs, or the source does not end.
    struct Teardown(mpsc::Receiver<()>);
    impl Drop for Teardown {
        fn drop(&mut self) {
            let (cleared, clears) = mpsc::channel();
            std::thread::spawn(move || for clipboard in [copy::ClipboardType::Regular, copy::ClipboardType::Primary] { cleared.send(copy::clear(clipboard, copy::Seat::All).is_ok()).ok(); });
            let until = crate::time::now() + Duration::from_secs(10);
            let left = || until.saturating_duration_since(crate::time::now());
            // Every wait runs, so a failed first clear still waits out the second and the source's end.
            let ended = [clears.recv_timeout(left()) == Ok(true), clears.recv_timeout(left()) == Ok(true), self.0.recv_timeout(left()).is_ok()];
            if !std::thread::panicking() { assert_eq!(ended, [true; 3], "both clears succeeded and the stalled source ended within 10 s"); }
        }
    }

    /// A provider that sends a prefix and keeps the pipe open costs one read deadline, not the
    /// worker: the read fails without its prefix, no later form is asked for, and the next regular
    /// and primary writes and reads complete once each, in order.
    #[test]
    #[ignore = "needs a Wayland compositor with data-control (WAYLAND_DISPLAY)"]
    fn live_wayland_stalled_provider_is_bounded() {
        use super::super::wayland::READ_FOR;
        use plain::{Completion, Operation, Outcome};
        use stalled::Seen;
        isolated();
        let watchdog = Duration::from_secs(30);
        let (seen, ended) = stalled::start();
        let _teardown = Teardown(ended);

        // One call's text and HTML forms share one deadline, and HTML is never requested.
        let (sent, got) = mpsc::channel();
        std::thread::spawn(move || {
            let start = crate::time::now();
            sent.send((Wayland.content(false).is_none(), start.elapsed())).ok();
        });
        let (none, took) = got.recv_timeout(watchdog).expect("a stalled content read ends within the watchdog");
        assert!(none, "a stalled read yields nothing, not its prefix");
        assert!(took >= READ_FOR, "the stalled read waited out its deadline: {took:?}");
        assert_eq!(seen.recv_timeout(watchdog), Ok(Seen::Sent("text/plain;charset=utf-8".into())));

        let worker = Worker::spawn(|| Some(Wayland));
        let (done, results) = mpsc::channel();
        let completion = |i: usize| {
            let done = done.clone();
            Completion(Some(Box::new(move |outcome| done.send((i, outcome, crate::time::now())).unwrap())))
        };
        let queued = crate::time::now();
        for request in [
            Operation::Read(false, 1 << 20, completion(0)),
            Operation::Write(false, "after-regular".into(), completion(1)),
            Operation::Write(true, "after-primary".into(), completion(2)),
            Operation::Read(false, 1 << 20, completion(3)),
            Operation::Read(true, 1 << 20, completion(4)),
        ] { worker.sender.send(Request::Plain(request)).unwrap(); }
        drop(done);
        let expected = [Outcome::HostFailure, Outcome::Written, Outcome::Written, Outcome::Text("after-regular".into()), Outcome::Text("after-primary".into())];
        for (i, want) in expected.into_iter().enumerate() {
            let (index, outcome, at) = results.recv_timeout(watchdog).unwrap_or_else(|e| panic!("completion {i} within the watchdog: {e}"));
            assert_eq!((index, outcome), (i, want), "completions in request order");
            if i == 0 { assert!(at - queued >= READ_FOR, "the stalled read waited out its deadline: {:?}", at - queued); }
        }
        assert_eq!(results.recv_timeout(watchdog).err(), Some(mpsc::RecvTimeoutError::Disconnected), "one completion per request");
        // The worker's read asked for text again: no HTML request came between the two.
        assert_eq!(seen.recv_timeout(watchdog), Ok(Seen::Sent("text/plain;charset=utf-8".into())));
        assert_eq!(seen.recv_timeout(Duration::from_secs(10)), Ok(Seen::Cancelled), "Rune's write took the selection");
        assert!(worker.stop(), "the worker acknowledged its cleanup");
    }
}
