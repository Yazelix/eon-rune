//! A Linux build without `x11` refuses a launch that has no Wayland display.

use super::event_loop::{WAYLAND_ONLY, wayland_only_refusal};
use std::ffi::OsString;

fn refusal(vars: &[(&str, &str)]) -> Option<String> {
    wayland_only_refusal(|name| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| OsString::from(v)))
}

#[test]
fn no_wayland_display_is_refused_whatever_x11_offers() {
    assert_eq!(refusal(&[]).as_deref(), Some(WAYLAND_ONLY));
    assert_eq!(refusal(&[("DISPLAY", ":0")]).as_deref(), Some(WAYLAND_ONLY), "DISPLAY alone is no Wayland display");
    assert_eq!(refusal(&[("WAYLAND_DISPLAY", ""), ("WAYLAND_SOCKET", "")]).as_deref(), Some(WAYLAND_ONLY), "empty counts as unset");
}

#[test]
fn either_wayland_variable_lets_winit_try() {
    assert_eq!(refusal(&[("WAYLAND_DISPLAY", "wayland-1")]), None);
    assert_eq!(refusal(&[("WAYLAND_SOCKET", "3")]), None);
}

#[test]
fn the_refusal_names_wayland_and_the_missing_x11_support() {
    assert!(WAYLAND_ONLY.starts_with("event loop: "));
    assert!(WAYLAND_ONLY.contains("WAYLAND_DISPLAY") && WAYLAND_ONLY.contains("no X11 support"));
}
