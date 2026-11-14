use super::*;

#[derive(Default)]
pub(crate) enum Policy {
    #[default]
    Recover,
    Terminal,
    Failed,
}

impl Policy {
    pub(super) fn failed_on(&self, gpu: &Gpu) -> bool {
        !matches!(self, Self::Recover) && (gpu.is_lost() || gpu.failure.message().is_some())
    }

    fn step(&mut self, reason: String) -> crate::machine::Effects<String> {
        if !matches!(self, Self::Terminal) {
            return crate::machine::Effects::none();
        }
        *self = Self::Failed;
        crate::machine::Effects::one(reason)
    }
}

impl<S: 'static> crate::App<S> {
    /// Give up permanently on GPU failure; `AppHandle::window_error` records why.
    pub fn terminal_gpu_loss(mut self) -> Self {
        self.hooks.gpu_failure = Policy::Terminal;
        self
    }
}

impl<S: 'static> Host<S> {
    pub(super) fn gpu_failed(&mut self) -> bool {
        match self.hooks.gpu_failure {
            Policy::Recover => return false,
            Policy::Failed => return true,
            Policy::Terminal => {}
        }
        let reason = self.gpu.as_ref().and_then(|(gpu, _)| {
            gpu.failure
                .message()
                .or_else(|| gpu.is_lost().then(|| "the GPU device was lost".into()))
        });
        if let Some(reason) = reason {
            return self.fail_gpu(reason);
        }
        false
    }

    pub(super) fn fail_gpu(&mut self, reason: String) -> bool {
        for reason in self.hooks.gpu_failure.step(reason) {
            self.give_up_surface(reason);
        }
        true
    }

    pub(super) fn discard_failed_frame(&mut self, frame: wgpu::SurfaceTexture) {
        // wgpu discards an unpresented frame through its surface's id.
        drop(frame);
        self.gpu_failed();
    }

    pub(super) fn terminal_surface(&mut self) -> bool {
        if self
            .win
            .as_mut()
            .is_none_or(|w| w.surface_retries.attempt() == SurfaceStep::GiveUp)
        {
            self.fail_gpu("surface replacement exhausted its retry limit".into());
            return false;
        }
        match self.same_device_surface() {
            Ok(()) => true,
            Err(reason) => {
                self.fail_gpu(reason);
                false
            }
        }
    }

    fn same_device_surface(&mut self) -> Result<(), String> {
        let (gpu, _) = self.gpu.as_ref().ok_or("there is no GPU to reconfigure")?;
        let w = self
            .win
            .as_mut()
            .ok_or("there is no window to reconfigure")?;
        w.presented = None;
        drop(w.surface.take());
        let surface = gpu
            .instance
            .create_surface(w.window.clone())
            .map_err(|e| e.to_string())?;
        configure(&surface, gpu, &w.config)?;
        #[cfg(target_os = "macos")]
        {
            crate::platform::macos::pin_contents_top_left(&surface);
            w.native.present = w.native.present_log.clone().and_then(|log| {
                crate::frame::present_time::metal::LayerHook::attach(&surface, log)
            });
            w.native.prime.stop();
        }
        w.surface = Some(surface);
        w.surface_ok = true;
        w.backoff_step(BackoffEvent::Lapsed);
        Ok(())
    }
}

#[cfg(test)]
#[path = "gpu_failure_tests.rs"]
pub(crate) mod tests;
