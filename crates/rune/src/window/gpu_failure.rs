use std::sync::Arc;

use crate::render::{Gpu, failure::Failure};

#[derive(Clone, Default)]
pub(crate) enum Policy {
    #[default]
    Recover,
    Terminal(Arc<Failure>),
    Failed,
}

impl Policy {
    pub(crate) fn watch_setup<T: Send + Sync + 'static>(
        &self, gpu: &Gpu, owner: &Arc<T>, notify: fn(&T),
    ) -> Result<(), crate::render::SurfaceSetupError> {
        if matches!(self, Self::Terminal(_)) { gpu.failure.watch(owner, notify).ok(); }
        self.observe_setup(Some(gpu))
    }

    pub(crate) fn observe_setup(&self, gpu: Option<&Gpu>) -> Result<(), crate::render::SurfaceSetupError> {
        self.observe(gpu).map_err(|message| crate::render::SurfaceSetupError {
            kind: crate::gpu::fallback::Failure::DeviceLost,
            message,
        })
    }

    pub(crate) fn configure_failed(&self, mut error: crate::render::SurfaceSetupError) -> crate::render::SurfaceSetupError {
        if matches!(self, Self::Terminal(_)) {
            self.record(&error.message);
            error.kind = crate::gpu::fallback::Failure::DeviceLost;
        }
        error
    }

    pub(crate) fn observe(&self, gpu: Option<&Gpu>) -> Result<(), String> {
        if matches!(self, Self::Failed) {
            return Err("the app's GPU failure is terminal".into());
        }
        if let Some(reason) = self.reason(gpu) {
            self.record(&reason);
            return Err(reason);
        }
        Ok(())
    }

    pub(crate) fn record(&self, reason: &str) {
        if let Self::Terminal(latch) = self { latch.record(reason.to_owned()); }
    }

    pub(crate) fn failed_on(&self, gpu: &Gpu) -> bool {
        matches!(self, Self::Failed) || self.reason(Some(gpu)).is_some()
    }

    pub(crate) fn reason(&self, gpu: Option<&Gpu>) -> Option<String> {
        let Self::Terminal(latch) = self else { return None };
        latch.message().or_else(|| gpu.and_then(|gpu| {
            gpu.failure.message().or_else(|| gpu.is_lost().then(|| "the GPU device was lost".into()))
        }))
    }

    pub(crate) fn stop(&mut self, reason: String) -> Option<String> {
        let Self::Terminal(latch) = self else { return None };
        latch.record(reason.clone());
        let reason = latch.message().unwrap_or(reason);
        *self = Self::Failed;
        Some(reason)
    }
}

impl<S: 'static> crate::App<S> {
    /// Give up permanently on native GPU failure. `AppHandle::window_error` reports the reason.
    pub fn terminal_gpu_loss(self) -> Self {
        let core = &self.shared.core;
        let latch = core.gpu_failure.get_or_init(Default::default);
        latch.watch(core, crate::app::SharedCore::wake_app).ok();
        self
    }
}

#[cfg(test)]
test_group! { native =>
#[path = "gpu_failure_tests.rs"]
mod tests;
}

impl super::host::Host {
    pub(in crate::window) fn gpu_failed(&mut self) -> bool {
        if matches!(self.gpu_failure, Policy::Failed) { return true; }
        let reason = self.gpu_failure.reason(self.gpu.as_ref().map(|(gpu, _)| gpu.as_ref()));
        #[cfg(target_os = "windows")]
        let reason = reason.or_else(|| self.pending_gpu_failure());
        if let Some(reason) = reason { self.fail_terminal_gpu(reason); return true; }
        false
    }

    pub(in crate::window) fn fail_terminal_gpu(&mut self, reason: String) {
        if let Some(reason) = self.gpu_failure.stop(reason) {
            self.give_up_surface(reason);
            self.publish();
        }
    }
}
