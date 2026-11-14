use super::*;

impl<S: 'static> Host<S> {
    pub(super) fn run_hook(&mut self, event_loop: &ActiveEventLoop, f: impl FnOnce(&mut S, &mut Tx) -> bool) -> bool {
        let shown = self.life.shown();
        let changed = self.rt.update_if(f);
        self.gpu_failed();
        self.rt.schedule_pending_wakes();
        self.clipboard.flush(&mut self.rt);
        self.shell(event_loop);
        let stopped = self.life.finished() || event_loop.exiting() || self.win.is_none() || (shown && !self.life.shown());
        if changed
            && !stopped
            && let Some(w) = &self.win
        {
            w.request_redraw();
        }
        stopped
    }

    // Device-loss guards run before the raw hook.
    pub(super) fn before_window_event(&mut self, event_loop: &ActiveEventLoop, event: &WindowEvent) -> bool {
        if self.gpu_failed() { return true; }
        let Some(f) = self.hooks.window_event else { return false };
        let Some(window) = self.win.as_ref().map(|w| w.window.clone()) else { return false };
        let mods = match event {
            WindowEvent::ModifiersChanged(m) => m.state(),
            _ => self.mods,
        };
        let mut taken = false;
        let stopped = self.run_hook(event_loop, |s, tx| {
            tx.mods = map_mods(mods);
            tx.key_repeat = matches!(event, WindowEvent::KeyboardInput { event, .. } if event.repeat);
            taken = f(s, &window, event, tx);
            taken && !tx.live_only
        });
        self.win.is_none() || ((stopped || taken) && consumable(event))
    }
}

fn consumable(event: &WindowEvent) -> bool {
    matches!(
        event,
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
#[path = "hooks_tests.rs"]
mod tests;
