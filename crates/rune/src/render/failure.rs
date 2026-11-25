use crate::runtime::Waker;
use std::sync::{Arc, Mutex, Weak};

#[derive(Default)]
pub(crate) struct Failure {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    message: Option<String>,
    watchers: Vec<Watcher>,
}

struct Watcher {
    // Key repeated surface setups by owner without keeping closed windows alive.
    owner: Weak<dyn Send + Sync>,
    wake: Waker,
}

impl Failure {
    pub(crate) fn record(&self, mut message: String) {
        let mut state = self.state.lock().unwrap();
        if state.message.is_some() { return; }
        message.truncate(message.floor_char_boundary(1024));
        state.message = Some(message);
        let watchers = std::mem::take(&mut state.watchers);
        drop(state);
        for watcher in watchers { (watcher.wake)(); }
    }

    pub(crate) fn watch<T: Send + Sync + 'static>(&self, owner: &Arc<T>, notify: fn(&T)) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if let Some(message) = &state.message { return Err(message.clone()); }
        let weak = Arc::downgrade(owner);
        let key: Weak<dyn Send + Sync> = weak.clone();
        state.watchers.retain(|watcher| watcher.owner.strong_count() != 0);
        if !state.watchers.iter().any(|watcher| watcher.owner.ptr_eq(&key)) {
            let wake = Arc::new(move || {
                if let Some(owner) = weak.upgrade() { notify(&owner); }
            });
            state.watchers.push(Watcher { owner: key, wake });
        }
        Ok(())
    }

    pub(crate) fn message(&self) -> Option<String> {
        self.state.lock().unwrap().message.clone()
    }
}

#[cfg(test)]
test_group! { paint =>
#[path = "gpu_failure_tests.rs"]
mod tests;
}
