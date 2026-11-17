use super::*;

#[cfg(all(test, target_os = "linux"))]
thread_local! {
    pub(crate) static REBLENDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Default)]
pub(crate) enum Policy {
    #[default]
    Recover,
    Terminal(Arc<crate::render::failure::Failure>),
    Failed,
}

impl Policy {
    pub(crate) fn watch(
        &self,
        gpu: &Gpu,
        wake: impl FnOnce() -> crate::runtime::Waker,
    ) -> Result<(), String> {
        if matches!(self, Self::Terminal(_)) {
            let _ = gpu.failure.watch(wake());
        }
        self.observe(Some(gpu))
    }

    /// Attribute an observed worker/device failure before its result can be discarded.
    pub(crate) fn observe(&self, gpu: Option<&Gpu>) -> Result<(), String> {
        match self {
            Self::Recover => Ok(()),
            Self::Failed => Err("the app's GPU failure is terminal".into()),
            Self::Terminal(_) => {
                if let Some(reason) = self.reason(gpu) {
                    self.record(&reason);
                    Err(reason)
                } else {
                    Ok(())
                }
            }
        }
    }

    pub(crate) fn record(&self, reason: &str) {
        if let Self::Terminal(latch) = self {
            latch.record(reason.to_owned());
        }
    }

    pub(crate) fn failed_on(&self, gpu: &Gpu) -> bool {
        matches!(self, Self::Failed) || self.reason(Some(gpu)).is_some()
    }

    pub(crate) fn reason(&self, gpu: Option<&Gpu>) -> Option<String> {
        let Self::Terminal(latch) = self else {
            return None;
        };
        latch.message().or_else(|| {
            gpu.and_then(|gpu| {
                gpu.failure
                    .message()
                    .or_else(|| gpu.is_lost().then(|| "the GPU device was lost".into()))
            })
        })
    }

    pub(crate) fn report(&mut self, reason: String) -> crate::machine::Effects<String> {
        self.record(&reason);
        self.step(reason)
    }

    fn step(&mut self, reason: String) -> crate::machine::Effects<String> {
        let Self::Terminal(latch) = self else {
            return crate::machine::Effects::none();
        };
        let reason = latch.message().unwrap_or(reason);
        *self = Self::Failed;
        crate::machine::Effects::one(reason)
    }
}

impl<S: 'static> crate::App<S> {
    /// Give up permanently on GPU failure; `AppHandle::window_error` records why.
    pub fn terminal_gpu_loss(mut self) -> Self {
        let latch = self
            .shared
            .gpu_failure
            .get_or_init(Default::default)
            .clone();
        let shared = Arc::downgrade(&self.shared);
        let _ = latch.watch(Arc::new(move || {
            if let Some(shared) = shared.upgrade() {
                shared.wake();
            }
        }));
        self.hooks.gpu_failure = Policy::Terminal(latch);
        self
    }
}

impl<S: 'static> Host<S> {
    pub(super) fn gpu_failed(&mut self) -> bool {
        match self.hooks.gpu_failure {
            Policy::Recover => return false,
            Policy::Failed => return true,
            Policy::Terminal(_) => {}
        }
        let reason = self
            .hooks
            .gpu_failure
            .reason(self.gpu.as_ref().map(|g| &*g.0));
        #[cfg(target_os = "windows")]
        let reason = reason.or_else(|| self.pending_gpu_failure());
        if let Some(reason) = reason {
            return self.fail_gpu(reason);
        }
        false
    }

    pub(super) fn fail_gpu(&mut self, reason: String) -> bool {
        for reason in self.hooks.gpu_failure.report(reason) {
            self.give_up_surface(reason);
            self.publish();
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
test_group! { native =>
#[path = "gpu_failure_tests.rs"]
pub(crate) mod tests;
}
