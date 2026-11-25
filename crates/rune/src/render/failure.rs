use crate::runtime::Waker;
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct Failure {
    message: Mutex<Option<String>>,
    wake: Mutex<Option<Waker>>,
}

impl Failure {
    pub(crate) fn record(&self, mut message: String) {
        let mut first = self.message.lock().unwrap();
        if first.is_some() { return; }
        message.truncate(message.floor_char_boundary(1024));
        *first = Some(message);
        drop(first);
        let wake = self.wake.lock().unwrap().clone();
        if let Some(wake) = wake { wake(); }
    }

    pub(crate) fn watch(&self, wake: Waker) -> Result<(), String> {
        *self.wake.lock().unwrap() = Some(wake);
        self.message().map_or(Ok(()), Err)
    }

    pub(crate) fn message(&self) -> Option<String> {
        self.message.lock().unwrap().clone()
    }
}

#[cfg(test)]
test_group! { paint =>
#[path = "gpu_failure_tests.rs"]
mod tests;
}
