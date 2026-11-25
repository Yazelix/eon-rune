use crate::{App, Tx};
use winit::{event::WindowEvent, window::Window};

/// Native size, scale, and output observation for [`App::on_window_geometry`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WindowGeometry {
    pub physical_size: (u32, u32),
    pub scale_factor: f64,
    pub has_output: bool,
}

pub(crate) type WindowEventHook<S> = fn(&mut S, &Window, &WindowEvent, &mut Tx) -> bool;
type GeometryHook<S> = fn(&mut S, WindowGeometry, &mut Tx) -> bool;

pub(crate) struct WindowHooks<S> {
    pub(crate) event: Option<WindowEventHook<S>>,
    pub(crate) geometry: Option<GeometryHook<S>>,
    pub(crate) last_geometry: Option<WindowGeometry>,
}

impl<S> Default for WindowHooks<S> {
    fn default() -> Self {
        Self { event: None, geometry: None, last_geometry: None }
    }
}

impl<S: 'static> App<S> {
    /// Observes native events before Rune handles them. Return true to consume an input event
    /// and mark state changed. Geometry, focus, redraw and modifier bookkeeping still run.
    /// Use `tx.live_only()` when the view state is unchanged. Native windows only.
    pub fn on_window_event(mut self, hook: WindowEventHook<S>) -> Self {
        self.window_hooks.event = Some(hook);
        self
    }

    /// Observes initial and changed idle native geometry. Return true when the view state changed.
    pub fn on_window_geometry(mut self, hook: GeometryHook<S>) -> Self {
        self.window_hooks.geometry = Some(hook);
        self
    }
}
