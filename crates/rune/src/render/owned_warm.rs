#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
use super::failure::Failure;
#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
use std::sync::Arc;

impl super::Gpu {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn for_owned_surface<'w>(
        policy: &crate::window::gpu_failure::Policy,
        wanted: wgpu::Features,
        translucent: bool,
        surface: impl FnOnce(&wgpu::Instance) -> Result<wgpu::Surface<'w>, String>,
    ) -> Result<(Self, wgpu::Surface<'w>, Option<super::Renderer>), String> {
        let early = super::prewarm::take();
        // Taking can wait for a worker that has just failed this owner.
        if let Some(reason) = policy.reason(None) {
            return Err(reason);
        }
        Self::for_surface_from(early, wanted, translucent, surface)
    }
}

impl<S: 'static> crate::AppHandle<S> {
    /// Warm a device and prepare compositor resources on a worker, attributed to this app.
    /// Under `terminal_gpu_loss`, a failed app starts no work and a GPU failure during
    /// preparation or observed when taking the waiting device permanently fails its window
    /// and owned overlays. Use an empty callback
    /// to warm only. Windows and Linux; elsewhere this does nothing.
    /// Without a live surface device, a making or waiting warm-up only gets a later
    /// deadline: its owner stays unchanged and this call's `prepare` is dropped.
    pub fn warm_gpu_with(
        &self,
        features: wgpu::Features,
        prepare: impl FnOnce(&wgpu::Device) + Send + 'static,
    ) {
        #[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
        super::prewarm::warm_for(
            features,
            Some(Box::new(prepare)),
            self.shared.gpu_failure.get().cloned(),
        );
        #[cfg(any(target_os = "macos", target_arch = "wasm32"))]
        let _ = (features, prepare);
    }
}

#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
pub(super) fn allowed(gpu: Option<&super::Gpu>, owner: &Option<Arc<Failure>>) -> bool {
    let Some(latch) = owner else { return true };
    crate::window::gpu_failure::Policy::Terminal(latch.clone())
        .observe(gpu)
        .is_ok()
}

#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
pub(super) fn prepare(
    gpu: &super::Gpu,
    prepare: Option<Box<dyn FnOnce(&wgpu::Device) + Send>>,
    owner: &Option<Arc<Failure>>,
) -> bool {
    if !allowed(Some(gpu), owner) {
        return false;
    }
    if let Some(prepare) = prepare {
        prepare(&gpu.device);
    }
    allowed(Some(gpu), owner)
}
