//! The Wayland clipboard over the data-control protocol (wl-clipboard-rs): the Linux backend of a
//! build without `x11`, and the bounded plain-text read every Linux build uses. No X11 fallback: a
//! compositor without data-control reads as unsupported and fails each write.
//!
//! A write hands its bytes to a wl-clipboard-rs thread that serves pastes until the compositor
//! cancels the offer (another copy, or `clear`), so the worker never waits on a paste. Reads open
//! their own connection, which that thread serves too. Rich reads are capped while reading, and a
//! refused one drains its pipe rather than closing it: wl-clipboard-rs ends an offer whose pipe
//! write fails, Rune's own included.
//! Every pipe a worker call reads is read by one deadline ([`READ_FOR`], [`Within`]): a provider
//! that stops sending costs that call, not the worker. `get_contents` waits only on the compositor.

use super::*;
use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd};
use std::path::PathBuf;
use wl_clipboard_rs::{copy, paste};

#[cfg_attr(rune_x11, allow(dead_code))]
const PNG: &str = "image/png";
#[cfg_attr(rune_x11, allow(dead_code))]
const HTML: &str = "text/html";
#[cfg_attr(rune_x11, allow(dead_code))]
const URI_LIST: &str = "text/uri-list";
/// Asks clipboard managers (KDE's Klipper and others) to keep a secret out of their history; the
/// type and value arboard writes for `exclude_from_history`.
#[cfg_attr(rune_x11, allow(dead_code))]
const SECRET_HINT: (&str, &[u8]) = ("x-kde-passwordManagerHint", b"secret");
/// How long one worker call may spend reading a provider's pipes, draining included.
pub(super) const READ_FOR: Duration = Duration::from_secs(5);

fn deadline() -> Instant { crate::time::now() + READ_FOR }

/// The bounded plain-text read: at most `cap` + 1 bytes of the best text type offered.
pub(super) fn bounded_text(primary: bool, cap: usize) -> plain::Outcome {
    use paste::{ClipboardType, Error, MimeType, Seat, get_mime_types_ordered};
    let until = deadline();
    let selection = if primary { ClipboardType::Primary } else { ClipboardType::Regular };
    let pipe = get_mime_types_ordered(selection, Seat::Unspecified).and_then(|types| plain::text_mime(&types).ok_or(Error::NoMimeType))
        .map_err(|e| read_outcome(&e)).and_then(|mime| open(selection, MimeType::Specific(mime), until));
    match pipe {
        Ok(pipe) => plain::decode(pipe, cap),
        Err(outcome) => outcome,
    }
}

/// One offered type's pipe, to be read by `until`; once `until` has passed none is opened.
pub(super) fn open(selection: paste::ClipboardType, mime: paste::MimeType<'_>, until: Instant) -> Result<Within<impl Read + AsFd>, plain::Outcome> {
    if crate::time::now() >= until { return Err(plain::Outcome::HostFailure); }
    let (pipe, _) = paste::get_contents(selection, paste::Seat::Unspecified, mime).map_err(|e| read_outcome(&e))?;
    Ok(Within { pipe, until })
}

/// What a deadline read does next: checked before every read, so a `poll` that wakes at or past
/// `until` (as from the rounded-up millisecond) fails.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Next { Read, Wait, TimedOut }
pub(super) fn next(now: Instant, until: Instant, ready: bool) -> Next {
    if now >= until { Next::TimedOut } else if ready { Next::Read } else { Next::Wait }
}

/// A pipe read that gives up at `until`: it waits for the pipe with `poll` and reads only once
/// the pipe is ready and `until` hasn't passed, so it never blocks past `until`. Rune is the
/// pipe's only reader, so a read of a ready pipe returns at once.
pub(super) struct Within<R> { pub(super) pipe: R, pub(super) until: Instant }
impl<R: AsFd> Within<R> {
    /// Waits up to the time left; true when the pipe is readable (bytes, its end, or an error).
    #[allow(unsafe_code)]
    fn wait(&self) -> io::Result<bool> {
        let left = self.until.saturating_duration_since(crate::time::now());
        // Rounded up, so a wait that times out never wakes before `until`.
        let ms = libc::c_int::try_from(left.as_nanos().div_ceil(1_000_000)).unwrap_or(libc::c_int::MAX);
        let mut fd = libc::pollfd { fd: self.pipe.as_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: `fd` is one valid pollfd that lives across the call, and the count is 1.
        match unsafe { libc::poll(&mut fd, 1, ms) } {
            -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => Ok(false),
            -1 => Err(io::Error::last_os_error()),
            _ => Ok(fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0),
        }
    }
}
impl<R: Read + AsFd> Read for Within<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut ready = false;
        loop {
            match next(crate::time::now(), self.until, ready) {
                Next::TimedOut => return Err(io::ErrorKind::TimedOut.into()),
                Next::Read => return self.pipe.read(buf),
                Next::Wait => ready = self.wait()?,
            }
        }
    }
}

/// What a failed read reports.
pub(super) fn read_outcome(error: &paste::Error) -> plain::Outcome {
    use paste::Error;
    match error {
        Error::ClipboardEmpty => plain::Outcome::Empty,
        Error::NoMimeType => plain::Outcome::NonText,
        Error::WaylandConnection(_) | Error::MissingProtocol { .. } | Error::PrimarySelectionUnsupported => plain::Outcome::Unsupported,
        _ => plain::Outcome::HostFailure,
    }
}

/// The clipboard through Wayland data-control. It holds nothing: each operation connects.
#[cfg(not(rune_x11))]
pub(super) struct Wayland;

#[cfg(not(rune_x11))]
impl Backend for Wayland {
    fn primary_text(&mut self) -> Option<String> { read_text(paste::ClipboardType::Primary, deadline()) }
    fn write_primary(&mut self, text: String) -> Result<(), String> {
        offer(copy::ClipboardType::Primary, text_sources(text.into_bytes(), false))
    }
    fn clear_primary(&mut self) -> Result<(), String> { clear(copy::ClipboardType::Primary) }
    fn bounded_text(&mut self, primary: bool, cap: usize) -> plain::Outcome { bounded_text(primary, cap) }
    fn write(&mut self, copy: &ClipboardWrite) -> Result<(), String> {
        offer(copy::ClipboardType::Regular, sources(copy)?)
    }
    fn text(&mut self) -> Option<Zeroizing<String>> { read_text(paste::ClipboardType::Regular, deadline()).map(Zeroizing::new) }
    /// Text, HTML, and with `all` the PNG image and the files; each capped as it's read, all by one deadline.
    fn content(&mut self, all: bool) -> Option<ClipboardContent> {
        let until = deadline();
        let text = read_text(paste::ClipboardType::Regular, until).map(Zeroizing::new);
        let html = read_capped(HTML, html::MAX_RICH_BYTES, until).and_then(|h| String::from_utf8(h).ok()).filter(|h| !h.is_empty());
        let image = if all { read_capped(PNG, MAX_IMAGE_BYTES, until).and_then(ClipboardImage::from_png) } else { None };
        let files = if all { read_capped(URI_LIST, html::MAX_RICH_BYTES, until).map(|list| uri_list_paths(&list)).unwrap_or_default() } else { Vec::new() };
        if text.is_none() && html.is_none() && image.is_none() && files.is_empty() { return None; }
        let mut content = ClipboardContent::from(text.unwrap_or_default()).with_files(files);
        if let Some(html) = html { content = content.with_html(html); }
        if let Some(image) = image { content = content.with_image(image); }
        Some(content)
    }
    /// Data-control has no change count; the hold watcher compares the text's fingerprint.
    fn revision(&self) -> Option<u64> { None }
    fn clear(&mut self) -> Result<(), String> { clear(copy::ClipboardType::Regular) }
}

/// The offers for one write, in the forms arboard's Wayland backend writes for the same call: a
/// rich write holds one form (the files, else the image, else HTML beside the text, else the text);
/// a secret write is its text with the history hint.
#[cfg_attr(rune_x11, allow(dead_code))]
pub(super) fn sources(write: &ClipboardWrite) -> Result<Vec<copy::MimeSource>, String> {
    let content = &write.content;
    if write.clear_after.is_some() || !content.is_rich() {
        return Ok(text_sources(content.text().as_bytes().to_vec(), write.clear_after.is_some()));
    }
    if !content.files().is_empty() {
        return Ok(vec![specific(URI_LIST, uri_list(content.files())?.into_bytes())]);
    }
    if let Some(image) = content.image() {
        return Ok(vec![specific(PNG, image.png().to_vec())]);
    }
    let mut sources = text_sources(content.text().as_bytes().to_vec(), false);
    if let Some(html) = content.html() { sources.push(specific(HTML, html.as_bytes().to_vec())); }
    Ok(sources)
}

#[cfg_attr(rune_x11, allow(dead_code))]
fn text_sources(text: Vec<u8>, secret: bool) -> Vec<copy::MimeSource> {
    let mut sources = vec![copy::MimeSource { source: copy::Source::Bytes(text.into_boxed_slice()), mime_type: copy::MimeType::Text }];
    if secret { sources.push(specific(SECRET_HINT.0, SECRET_HINT.1.to_vec())); }
    sources
}

#[cfg_attr(rune_x11, allow(dead_code))]
fn specific(mime: &str, bytes: Vec<u8>) -> copy::MimeSource {
    copy::MimeSource { source: copy::Source::Bytes(bytes.into_boxed_slice()), mime_type: copy::MimeType::Specific(mime.to_owned()) }
}

/// `files` as a `text/uri-list`, one canonical `file://` URI per line, CRLF between them (RFC 2483).
/// Files that no longer exist are left out, as arboard leaves them out; none left is an error.
#[cfg_attr(rune_x11, allow(dead_code))]
pub(super) fn uri_list(files: &[PathBuf]) -> Result<String, String> {
    let uris: Vec<String> = files.iter().filter_map(|f| f.canonicalize().ok()).map(|f| file_uri(&f)).collect();
    if uris.is_empty() { return Err("none of the copied files exists".into()); }
    Ok(uris.join("\r\n"))
}

/// The local files a `text/uri-list` names (RFC 2483): CRLF or LF lines, `#` comments and blank
/// lines skipped, and URIs that aren't local files left out.
#[cfg_attr(rune_x11, allow(dead_code))]
pub(super) fn uri_list_paths(list: &[u8]) -> Vec<PathBuf> {
    crate::platform::wayland_drop::uri::parse_uri_list(list)
}

/// `path` as a `file://` URI. Controls, bytes past ASCII and the characters a URI reserves or
/// forbids in a path are percent-encoded: the set arboard writes, so readers that took its file
/// lists take these.
#[cfg_attr(rune_x11, allow(dead_code))]
fn file_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if !(0x20..0x7f).contains(&b) || b"#;?[] \"%<>\\^`{|}".contains(&b) {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

#[cfg(not(rune_x11))]
fn offer(clipboard: copy::ClipboardType, sources: Vec<copy::MimeSource>) -> Result<(), String> {
    let mut options = copy::Options::new();
    options.clipboard(clipboard);
    options.copy_multi(sources).map_err(|e| e.to_string())
}

#[cfg(not(rune_x11))]
fn clear(clipboard: copy::ClipboardType) -> Result<(), String> {
    copy::clear(clipboard, copy::Seat::All).map_err(|e| e.to_string())
}

#[cfg(not(rune_x11))]
fn read_text(selection: paste::ClipboardType, until: Instant) -> Option<String> {
    let mut pipe = open(selection, paste::MimeType::Text, until).ok()?;
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

/// One type's bytes from the regular clipboard, or `None` when it isn't offered, the read fails
/// or runs out of time, or it's over `cap`.
#[cfg(not(rune_x11))]
fn read_capped(mime: &str, cap: usize, until: Instant) -> Option<Vec<u8>> {
    let mut pipe = open(paste::ClipboardType::Regular, paste::MimeType::Specific(mime), until).ok()?;
    match capped(&mut pipe, cap)? {
        Capped::Fits(bytes) => Some(bytes),
        Capped::Over => {
            drain(&mut pipe);
            None
        }
    }
}

/// A read stopped at its cap.
#[cfg_attr(rune_x11, allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Capped {
    Fits(Vec<u8>),
    /// More than the cap was offered; the reader is left just past it.
    Over,
}

#[cfg_attr(rune_x11, allow(dead_code))]
/// Reads at most `cap` + 1 bytes: all of them when they fit, else `Over`. `None` on a read error.
pub(super) fn capped(input: &mut impl Read, cap: usize) -> Option<Capped> {
    let mut bytes = Vec::new();
    input.take(cap.saturating_add(1) as u64).read_to_end(&mut bytes).ok()?;
    Some(if bytes.len() > cap { Capped::Over } else { Capped::Fits(bytes) })
}

#[cfg_attr(rune_x11, allow(dead_code))]
/// Reads `pipe` to its end without keeping it, by its deadline, so the offer's writer sees every
/// byte taken. A writer still going at the deadline has its pipe closed.
pub(super) fn drain(pipe: &mut Within<impl Read + AsFd>) {
    io::copy(pipe, &mut io::sink()).ok();
}
