use super::*;

pub(super) struct HostHooks<S> {
    app: crate::app::hooks::AppHooks<S>,
    _quit: Option<crate::quit::request::Registration>,
}

impl<S> From<crate::app::hooks::AppHooks<S>> for HostHooks<S> {
    fn from(app: crate::app::hooks::AppHooks<S>) -> Self {
        let registration = app
            .quit_request
            .map(|_| crate::quit::request::Registration::new());
        Self {
            app,
            _quit: registration,
        }
    }
}

impl<S> std::ops::Deref for HostHooks<S> {
    type Target = crate::app::hooks::AppHooks<S>;
    fn deref(&self) -> &Self::Target {
        &self.app
    }
}

impl<S> std::ops::DerefMut for HostHooks<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.app
    }
}

fn deliver_quit<S>(
    f: fn(&mut S, &mut Tx) -> crate::QuitResponse,
    state: &mut S,
    tx: &mut Tx,
) -> bool {
    crate::quit::request::answer(f(state, tx));
    !tx.live_only
}

impl<S: 'static> Host<S> {
    pub(super) fn quit_requested(&mut self, event_loop: &ActiveEventLoop) {
        if !self.started || self.life.finished() || event_loop.exiting() {
            return;
        }
        let Some(f) = self.hooks.quit_request else {
            return;
        };
        self.run_hook(event_loop, |s, tx| deliver_quit(f, s, tx));
    }

    /// True when the native request was answered, including a close carried out by its Tx.
    pub(super) fn close_requested(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let Some(f) = self.hooks.close_request else {
            return false;
        };
        if !self.closes() || event_loop.exiting() {
            return true;
        }
        self.run_hook(event_loop, |s, tx| {
            if f(s, tx) == crate::CloseResponse::Close {
                tx.close_window();
            }
            !tx.live_only
        });
        // A deferred request cannot lend its last-window hint to a later close.
        self.ends_app = false;
        true
    }

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
test_group! { native =>
#[path = "hooks_tests.rs"]
mod tests;
}

#[cfg(test)]
test_group! { native =>
#[path = "close_tests.rs"]
mod close_tests;
}

#[cfg(test)]
test_group! { native =>
#[path = "quit_request_tests.rs"]
mod quit_request_tests;
}

#[cfg(test)]
test_group! { native =>
#[cfg(target_os = "linux")]
#[path = "consent_lifecycle_tests.rs"]
mod consent_lifecycle_tests;
}
