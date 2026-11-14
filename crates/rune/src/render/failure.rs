use crate::runtime::Waker;
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct Failure {
    message: Mutex<Option<String>>,
    // One wake starts a turn for every host sharing this event loop/device.
    wake: Mutex<Option<Waker>>,
}

impl Failure {
    pub(crate) fn record(&self, mut message: String) {
        let mut first = self.message.lock().unwrap();
        if first.is_some() {
            return;
        }
        message.truncate(message.floor_char_boundary(1024));
        *first = Some(message);
        drop(first);
        let wake = self.wake.lock().unwrap().clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    pub(crate) fn watch(&self, wake: Waker) -> Result<(), String> {
        *self.wake.lock().unwrap() = Some(wake);
        self.message().map_or(Ok(()), Err)
    }

    pub(crate) fn message(&self) -> Option<String> {
        self.message.lock().unwrap().clone()
    }
}

impl super::Renderer {
    pub(crate) fn submit_if(
        &self,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        allowed: impl Fn() -> bool,
    ) -> Option<wgpu::SubmissionIndex> {
        self.before_submit(&mut encoder);
        if !allowed() {
            return None;
        }
        if let Some(surfaces) = &self.surfaces {
            surfaces.submit_if(queue, encoder, allowed)
        } else {
            let commands = encoder.finish();
            allowed().then(|| queue.submit([commands]))
        }
    }
}

#[cfg(test)]
#[path = "gpu_failure_tests.rs"]
mod tests;
