use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WindowGeometry {
    pub physical_size: (u32, u32),
    pub scale_factor: f64,
    pub has_output: bool,
}

impl<S: 'static> crate::App<S> {
    /// Initial and changed idle native geometry; true marks view state changed. Native only.
    pub fn on_window_geometry(mut self, f: fn(&mut S, WindowGeometry, &mut Tx) -> bool) -> Self {
        self.hooks.geometry = Some(f);
        self
    }
}

impl<S: 'static> Host<S> {
    pub(super) fn sample_geometry(&mut self, event_loop: &ActiveEventLoop, initial: bool) -> bool {
        if self.gpu_failed() { return true; }
        let Some(f) = self.hooks.geometry else { return false };
        let Some(w) = &self.win else { return false };
        let size = w.window.inner_size();
        let geometry = WindowGeometry {
            physical_size: (size.width, size.height),
            scale_factor: w.window.scale_factor(),
            has_output: w.window.current_monitor().is_some(),
        };
        if !initial && self.hooks.last_geometry == Some(geometry) {
            return false;
        }
        self.hooks.last_geometry = Some(geometry);
        self.run_hook(event_loop, |s, tx| f(s, geometry, tx))
    }
}

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod tests;
