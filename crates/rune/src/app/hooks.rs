use crate::{App, Tx};
use winit::{event::WindowEvent, window::Window};

pub(crate) type WindowEventHook<S> = fn(&mut S, &Window, &WindowEvent, &mut Tx) -> bool;

/// An application's answer to a native window close request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseResponse {
    /// Close normally, delivering `App::on_close` once.
    Close,
    /// Keep the window open; the app may close it programmatically later.
    Defer,
}

/// An application's answer to a macOS application Quit request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitResponse {
    /// Allow AppKit to terminate, delivering `App::on_quit` once.
    Quit,
    /// Cancel this native attempt; the app may quit programmatically later.
    Defer,
}

pub(crate) struct AppHooks<S> {
    pub(crate) close_request: Option<fn(&mut S, &mut Tx) -> CloseResponse>,
    pub(crate) quit_request: Option<fn(&mut S, &mut Tx) -> QuitResponse>,
    pub(crate) gpu_failure: crate::window::gpu_failure::Policy,
    pub(crate) window_event: Option<WindowEventHook<S>>,
    pub(crate) geometry: Option<fn(&mut S, crate::WindowGeometry, &mut Tx) -> bool>,
    // Cache of the last native observation.
    pub(crate) last_geometry: Option<crate::WindowGeometry>,
}

impl<S> Default for AppHooks<S> {
    fn default() -> Self {
        Self { close_request: None, quit_request: None, window_event: None, geometry: None, last_geometry: None, gpu_failure: Default::default() }
    }
}

impl<S: 'static> App<S> {
    /// Decide macOS application Quit before AppKit terminates. `Defer` cancels this attempt;
    /// resolve later with `Tx::quit` or `AppHandle::quit`. Keep the callback nonblocking.
    /// Hosts are asked in order until one defers. State changes redraw unless `tx.live_only()`.
    /// Programmatic quit, relaunch, last-window exit and teardown bypass consent.
    /// `on_quit` remains notification. Other platforms do not invoke this hook.
    /// Bundle opt-ins to Sudden or Automatic Termination can bypass the delegate entirely.
    pub fn on_quit_request(mut self, f: fn(&mut S, &mut Tx) -> QuitResponse) -> Self {
        self.hooks.quit_request = Some(f);
        self
    }

    /// Decide a native close request before Rune hides or changes the window's close state.
    /// `Defer` keeps it open; resolve later with `Tx::close_window` or a handle's `close`.
    /// State changes redraw unless `tx.live_only()` is set. Keep this callback nonblocking.
    /// Programmatic close, quit and teardown bypass it; `on_close` remains teardown notification.
    /// `Tx::quit` and `AppHandle::quit` skip `on_close`. With this hook registered, so does
    /// a native request preempted by an exiting loop.
    /// Native windows only. Rune's drawn Windows close button and macOS menu/shortcut close
    /// currently use programmatic close, so they also bypass this callback.
    pub fn on_close_request(mut self, f: fn(&mut S, &mut Tx) -> CloseResponse) -> Self {
        self.hooks.close_request = Some(f);
        self
    }

    /// Observe native events before Rune handles them; true consumes input and marks state
    /// changed. Geometry, focus, redraws and modifier bookkeeping cannot be consumed.
    /// Use `tx.live_only()` when no view state changed. Native windows only.
    pub fn on_window_event(mut self, f: WindowEventHook<S>) -> Self {
        self.hooks.window_event = Some(f);
        self
    }
}
