use super::*;
use crate::window::gpu_failure::Policy;

impl<T: 'static> Overlay<T> {
    pub(super) fn gpu_failed(&mut self) -> bool {
        if matches!(self.gpu_failure, Policy::Failed) {
            return true;
        }
        let Some(reason) = self.gpu_failure.reason(self.gpu.as_deref()) else {
            return false;
        };
        self.fail_gpu(reason);
        true
    }

    pub(super) fn fail_gpu(&mut self, reason: String) {
        for reason in self.gpu_failure.report(reason) {
            self.hide();
            self.release();
            self.renderer = None;
            self.gpu = None;
            self.fail(reason);
        }
    }

    pub(super) fn discard_failed_frame(&mut self, frame: wgpu::SurfaceTexture) -> bool {
        // The drawable must go before terminal teardown releases its surface.
        drop(frame);
        self.gpu_failed();
        false
    }
}

#[cfg(test)]
#[path = "gpu_failure_tests.rs"]
mod tests;
