use crate::{App, Tx};
use winit::{event::WindowEvent, window::Window};

pub(crate) type WindowEventHook<S> = fn(&mut S, &Window, &WindowEvent, &mut Tx) -> bool;

pub(crate) struct AppHooks<S> {
    pub(crate) window_event: Option<WindowEventHook<S>>,
    pub(crate) geometry: Option<fn(&mut S, crate::WindowGeometry, &mut Tx) -> bool>,
    // Cache of the last native observation.
    pub(crate) last_geometry: Option<crate::WindowGeometry>,
}

impl<S> Default for AppHooks<S> {
    fn default() -> Self {
        Self { window_event: None, geometry: None, last_geometry: None }
    }
}

impl<S: 'static> App<S> {
    /// Observe native events before Rune handles them; true consumes input and marks state
    /// changed. Geometry, focus, redraws and modifier bookkeeping cannot be consumed.
    /// Use `tx.live_only()` when no view state changed. Native windows only.
    pub fn on_window_event(mut self, f: WindowEventHook<S>) -> Self {
        self.hooks.window_event = Some(f);
        self
    }
}
