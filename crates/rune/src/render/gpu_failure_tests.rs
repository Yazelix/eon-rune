use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn first_failure_is_bounded_on_a_character_boundary_and_wakes_once() {
    let failure = Failure::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    failure
        .watch(Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }))
        .unwrap();
    assert!(failure.message().is_none());
    failure.record("界".repeat(400));
    assert_eq!(failure.message().unwrap(), "界".repeat(341));
    failure.record("later device loss".into());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(failure.message().unwrap().len(), 1023);
}

#[test]
fn subscription_observes_an_existing_failure_without_a_lost_wake() {
    let failure = Failure::default();
    failure.record("earlier failure".into());
    assert_eq!(
        failure.watch(Arc::new(|| panic!(
            "already failed subscription returns synchronously"
        ))),
        Err("earlier failure".into())
    );
    failure.record("duplicate".into());
    assert_eq!(failure.message().as_deref(), Some("earlier failure"));
}

#[test]
fn wake_runs_without_holding_the_failure_or_wake_mutex() {
    let failure = Arc::new(Failure::default());
    let weak = Arc::downgrade(&failure);
    failure
        .watch(Arc::new(move || {
            let failure = weak.upgrade().unwrap();
            assert_eq!(failure.message().as_deref(), Some("lost"));
            assert_eq!(failure.watch(Arc::new(|| {})), Err("lost".into()));
        }))
        .unwrap();
    failure.record("lost".into());
}

#[test]
fn callback_thread_can_publish_to_an_idle_host_waker() {
    let failure = Arc::new(Failure::default());
    let (send, recv) = std::sync::mpsc::channel();
    failure
        .watch(Arc::new(move || send.send(()).unwrap()))
        .unwrap();
    let worker = failure.clone();
    let thread = std::thread::spawn(move || worker.record("worker loss".into()));
    recv.recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    thread.join().unwrap();
    assert_eq!(failure.message().as_deref(), Some("worker loss"));
}
