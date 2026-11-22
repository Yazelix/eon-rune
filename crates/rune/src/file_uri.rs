//! Local `file://` URIs, as the XDG file chooser portal answers and as `text/uri-list` carries
//! files on the Wayland clipboard: a path's bytes percent-encoded, and back.

use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// A local `file://` URI (no host, or `localhost`) as a path, percent-decoded to raw bytes. Any
/// query or fragment is dropped. `None` for another scheme or a remote host.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(at) if rest[..at].eq_ignore_ascii_case("localhost") => &rest[at..],
        _ => return None,
    };
    let bytes = path.split(['?', '#']).next().unwrap_or_default().as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// `path` as a `file://` URI. Controls, bytes past ASCII and the characters a URI reserves or
/// forbids in a path are percent-encoded: the set arboard writes, so readers that took its file
/// lists take these.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn uri(path: &Path) -> String {
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
