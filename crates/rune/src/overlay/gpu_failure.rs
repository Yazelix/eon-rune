use super::{OverlayApp, OverlayCore};
use crate::window::gpu_failure::Policy;

impl OverlayCore {
    pub(super) fn gpu_failed(&mut self, app: &mut dyn OverlayApp) -> bool {
        if matches!(self.gpu_failure, Policy::Failed) { return true; }
        let Some(reason) = self.gpu_failure.reason(self.gpu.as_deref()) else { return false };
        self.fail_gpu(app, reason);
        true
    }

    pub(super) fn fail_gpu(&mut self, app: &mut dyn OverlayApp, reason: String) {
        if let Some(reason) = self.gpu_failure.stop(reason) {
            self.hide(app);
            self.release();
            self.renderer = None;
            self.gpu = None;
            app.release_compositor_gpu();
            self.fail(app, reason);
        }
    }
}

#[cfg(test)]
test_group! { native =>
#[cfg(target_os = "linux")]
#[path = "gpu_failure_native_tests.rs"]
mod tests;
}
