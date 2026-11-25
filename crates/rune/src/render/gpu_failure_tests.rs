use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn first_failure_is_bounded_on_a_character_boundary_and_wakes_once() {
    let failure = Failure::default();
    let calls = Arc::new(AtomicUsize::new(0));
    failure.watch(&calls, count_wake).unwrap();
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
        failure.watch(&Arc::new(()), |_| panic!(
            "already failed subscription returns synchronously"
        )),
        Err("earlier failure".into())
    );
    failure.record("duplicate".into());
    assert_eq!(failure.message().as_deref(), Some("earlier failure"));
}

#[test]
fn wake_runs_without_holding_the_failure_mutex() {
    let failure = Arc::new(Failure::default());
    let owner = Arc::new(Arc::downgrade(&failure));
    failure
        .watch(&owner, |weak| {
            let failure = weak.upgrade().unwrap();
            assert_eq!(failure.message().as_deref(), Some("lost"));
            assert_eq!(failure.watch(&Arc::new(()), |_| {}), Err("lost".into()));
        })
        .unwrap();
    failure.record("lost".into());
}

#[test]
fn callback_thread_can_publish_to_an_idle_host_waker() {
    let failure = Arc::new(Failure::default());
    let (send, recv) = std::sync::mpsc::channel();
    let send = Arc::new(send);
    failure
        .watch(&send, |send| send.send(()).unwrap())
        .unwrap();
    let worker = failure.clone();
    let thread = std::thread::spawn(move || worker.record("worker loss".into()));
    recv.recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    thread.join().unwrap();
    assert_eq!(failure.message().as_deref(), Some("worker loss"));
}

#[test]
fn device_failure_wakes_every_registered_terminal_owner() {
    let failure = Failure::default();
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(());
    failure.watch(&dropped, |_| panic!("a dropped owner cannot wake")).unwrap();
    drop(dropped);
    for owner in [&first, &second, &first] {
        failure.watch(owner, count_wake).unwrap();
    }

    failure.record("lost".into());
    assert_eq!(first.load(Ordering::SeqCst), 1, "first terminal window must still wake");
    assert_eq!(second.load(Ordering::SeqCst), 1, "second terminal window must wake");
}

fn count_wake(calls: &AtomicUsize) {
    calls.fetch_add(1, Ordering::SeqCst);
}
