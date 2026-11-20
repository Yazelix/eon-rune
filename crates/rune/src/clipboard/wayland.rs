//! The Wayland clipboard over the data-control protocol (wl-clipboard-rs): the Linux backend of a
//! build without `x11`, and the bounded plain-text read every Linux build uses. No X11 fallback: a
//! compositor without data-control reads as unsupported and fails each write.
//!
//! A write hands its bytes to a wl-clipboard-rs thread that serves pastes until the compositor
//! cancels the offer (another copy, or `clear`), so the worker never waits on a paste. Reads open
//! their own connection, which that thread serves too. Rich reads are capped while reading, and a
//! refused one drains its pipe rather than closing it: wl-clipboard-rs ends an offer whose pipe
//! write fails, Rune's own included.

use super::*;
use std::io::Read;
use std::os::fd::AsRawFd;
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
/// How long a refused rich read keeps draining its pipe so the offer survives.
#[cfg_attr(rune_x11, allow(dead_code))]
const DRAIN_FOR: Duration = Duration::from_secs(2);

/// The bounded plain-text read: at most `cap` + 1 bytes of the best text type offered.
pub(super) fn bounded_text(primary: bool, cap: usize) -> plain::Outcome {
    use paste::{ClipboardType, Error, MimeType, Seat, get_contents, get_mime_types_ordered};
    let selection = if primary { ClipboardType::Primary } else { ClipboardType::Regular };
    let contents = get_mime_types_ordered(selection, Seat::Unspecified).and_then(|types| plain::text_mime(&types).ok_or(Error::NoMimeType))
        .and_then(|mime| get_contents(selection, Seat::Unspecified, MimeType::Specific(mime)));
    match contents {
        Ok((pipe, _)) => plain::decode(pipe, cap),
        Err(e) => read_outcome(&e),
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
    fn primary_text(&mut self) -> Option<String> { read_text(paste::ClipboardType::Primary) }
    fn write_primary(&mut self, text: String) -> Result<(), String> {
        offer(copy::ClipboardType::Primary, text_sources(text.into_bytes(), false))
    }
    fn clear_primary(&mut self) -> Result<(), String> { clear(copy::ClipboardType::Primary) }
    fn bounded_text(&mut self, primary: bool, cap: usize) -> plain::Outcome { bounded_text(primary, cap) }
    fn write(&mut self, copy: &ClipboardWrite) -> Result<(), String> {
        offer(copy::ClipboardType::Regular, sources(copy)?)
    }
    fn text(&mut self) -> Option<Zeroizing<String>> { read_text(paste::ClipboardType::Regular).map(Zeroizing::new) }
    /// Text, HTML, and with `all` the PNG image and the files; each capped as it's read.
    fn content(&mut self, all: bool) -> Option<ClipboardContent> {
        let text = self.text();
        let html = read_capped(HTML, html::MAX_RICH_BYTES).and_then(|h| String::from_utf8(h).ok()).filter(|h| !h.is_empty());
        let image = if all { read_capped(PNG, MAX_IMAGE_BYTES).and_then(ClipboardImage::from_png) } else { None };
        let files = if all { read_capped(URI_LIST, html::MAX_RICH_BYTES).map(|list| uri_list_paths(&list)).unwrap_or_default() } else { Vec::new() };
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
    let uris: Vec<String> = files.iter().filter_map(|f| f.canonicalize().ok()).map(|f| crate::file_uri::uri(&f)).collect();
    if uris.is_empty() { return Err("none of the copied files exists".into()); }
    Ok(uris.join("\r\n"))
}

/// The local files a `text/uri-list` names (RFC 2483): CRLF or LF lines, `#` comments and blank
/// lines skipped, and URIs that aren't local files left out.
#[cfg_attr(rune_x11, allow(dead_code))]
pub(super) fn uri_list_paths(list: &[u8]) -> Vec<PathBuf> {
    list.split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.is_empty() && !line.starts_with(b"#"))
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(crate::file_uri::path)
        .collect()
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
fn read_text(selection: paste::ClipboardType) -> Option<String> {
    let (mut pipe, _) = paste::get_contents(selection, paste::Seat::Unspecified, paste::MimeType::Text).ok()?;
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

/// One type's bytes from the regular clipboard, or `None` when it isn't offered, the read fails,
/// or it's over `cap`.
#[cfg(not(rune_x11))]
fn read_capped(mime: &str, cap: usize) -> Option<Vec<u8>> {
    let (mut pipe, _) = paste::get_contents(paste::ClipboardType::Regular, paste::Seat::Unspecified, paste::MimeType::Specific(mime)).ok()?;
    match capped(&mut pipe, cap)? {
        Capped::Fits(bytes) => Some(bytes),
        Capped::Over => {
            drain(&mut pipe, DRAIN_FOR);
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
/// Reads `pipe` to its end without keeping it, for at most `limit`, so the offer's writer sees
/// every byte taken. A writer still going at the limit has its pipe closed.
pub(super) fn drain(pipe: &mut (impl Read + AsRawFd), limit: Duration) {
    let until = Instant::now() + limit;
    let mut scratch = vec![0u8; 64 * 1024];
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() { return; }
        let mut fd = libc::pollfd { fd: pipe.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // Rounded up, so a poll that times out never ends the wait before `until`.
        let ms = libc::c_int::try_from(left.as_micros().div_ceil(1000)).unwrap_or(libc::c_int::MAX);
        // SAFETY: `fd` is one valid pollfd that lives across the call, and the count is 1.
        let ready = unsafe { libc::poll(&mut fd, 1, ms) };
        if ready < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted { continue; }
        if ready < 0 { return; }
        // Timed out: the deadline check at the top decides whether to wait again.
        if ready == 0 { continue; }
        match pipe.read(&mut scratch) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}
