use winit::{event::WindowEvent, event_loop::ActiveEventLoop};

use super::Host;
use crate::window::{chrome::sync_chrome_core, keys::map_mods, sync_screen_origin_core};

impl Host {
    fn finish_window_hook(&mut self, event_loop: &ActiveEventLoop, was_shown: bool, stale: bool) -> bool {
        self.gpu_failed();
        self.app.rt_mut().schedule_pending_wakes();
        self.clipboard.flush_core(self.app.rt_mut());
        self.shell(event_loop);
        let stopped = self.life.finished() || event_loop.exiting() || self.win.is_none()
            || (was_shown && !self.life.shown());
        if stale && !stopped && let Some(w) = &self.win {
            w.request_redraw();
        }
        stopped
    }

    pub(super) fn before_window_event(&mut self, event_loop: &ActiveEventLoop, event: &WindowEvent) -> bool {
        let Some(w) = &self.win else { return false };
        let mods = match event {
            WindowEvent::ModifiersChanged(modifiers) => modifiers.state(),
            _ => self.mods,
        };
        let was_shown = self.life.shown();
        let Some((consumed, stale)) = self.app.run_window_event(&w.window, event, map_mods(mods)) else { return false };
        if consumed && consumable(event) && let Some(w) = self.win.as_mut() {
            w.timer_frame.no_longer_quiet();
            w.loop_frame.touched();
        }
        let stopped = self.finish_window_hook(event_loop, was_shown, stale);
        self.win.is_none() || ((stopped || consumed) && consumable(event))
    }

    pub(super) fn sample_geometry(&mut self, event_loop: &ActiveEventLoop, initial: bool) -> bool {
        if self.gpu_failed() { return true; }
        let Some(w) = &self.win else { return false };
        let was_shown = self.life.shown();
        let Some(stale) = self.app.run_window_geometry(&w.window, initial) else { return false };
        self.finish_window_hook(event_loop, was_shown, stale)
    }

    pub(super) fn native_quit_request(&mut self, event_loop: &ActiveEventLoop) {
        if self.life.finished() { return; }
        let was_shown = self.life.shown();
        let Some((response, stale)) = self.app.run_on_quit_request() else { return };
        crate::quit::request::answer(response);
        self.finish_window_hook(event_loop, was_shown, stale);
    }

    /// Notify before a first draw or focus. Even a hook that hides a retained window
    /// leaves it with a viewport matched to its native size.
    pub(in crate::window) fn finish_creation(&mut self, event_loop: &ActiveEventLoop) -> bool {
        if self.gpu_failed() { return true; }
        crate::marks::mark("window.created");
        if let Some(created) = self.on_created.take() { created(self.native_handle.clone()); }
        if let Some(on_window) = self.on_window.as_mut() { on_window(&self.native_handle); }
        let stopped = self.sample_geometry(event_loop, true);
        if self.life.finished() || event_loop.exiting() { return true; }
        let Some(w) = self.win.as_mut() else { return true };
        #[cfg(target_os = "macos")]
        if let Some(pacing) = &mut w.native.pacing { pacing.update_display(&w.window); }
        let size = w.window.inner_size();
        let scale = w.window.scale_factor() as f32;
        let rt = self.app.rt_mut();
        if size.width > 0 && size.height > 0 {
            let gpu = w.surface.as_ref().and(self.gpu.as_ref()).map(|(gpu, ..)| &**gpu);
            w.resize_surface(gpu, size);
            rt.set_viewport((w.config.width as f32 / scale, w.config.height as f32 / scale), scale);
        }
        sync_chrome_core(rt, w, self.chrome, self.titlebar);
        sync_screen_origin_core(rt, &w.window);
        stopped
    }
}

fn consumable(event: &WindowEvent) -> bool {
    matches!(event,
        WindowEvent::KeyboardInput { .. }
            | WindowEvent::Ime(_)
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::AxisMotion { .. }
    )
}

#[cfg(test)]
test_group! { native =>
#[path = "hook_tests.rs"]
mod tests;
}

#[cfg(test)]
test_group! { native =>
#[path = "geometry_tests.rs"]
mod geometry_tests;
}

#[cfg(test)]
test_group! { native =>
#[cfg(target_os = "linux")]
#[path = "gpu_failure_native_tests.rs"]
mod gpu_failure_tests;
}

#[cfg(test)]
test_group! { native =>
#[cfg(target_os = "linux")]
#[path = "attention_native_tests.rs"]
mod attention_tests;
}

#[cfg(test)]
test_group! { native =>
#[cfg(target_os = "linux")]
#[path = "skip_reason_native_tests.rs"]
mod skip_reason_native;
}
