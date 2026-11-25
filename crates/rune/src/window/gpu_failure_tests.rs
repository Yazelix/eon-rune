use super::*;

#[test]
fn terminal_transition_reports_the_first_failure_and_stops_once() {
    let latch = Arc::new(Failure::default());
    let mut policy = Policy::Terminal(latch.clone());
    policy.record("worker failed");
    assert_eq!(policy.observe(None), Err("worker failed".into()));
    assert!(matches!(policy, Policy::Terminal(_)));
    assert_eq!(policy.stop("later failure".into()).as_deref(), Some("worker failed"));
    assert!(matches!(policy, Policy::Failed));
    assert_eq!(policy.stop("duplicate".into()), None);
    assert!(policy.observe(None).is_err());
    assert_eq!(latch.message().as_deref(), Some("worker failed"));
}

#[test]
fn recover_is_the_default_and_handle_sees_its_opted_in_latch() {
    let mut policy = Policy::default();
    assert_eq!(policy.stop("validation".into()), None);
    assert!(matches!(policy, Policy::Recover));
    let app = crate::App::new(crate::AppSpec::new("default", (30., 30.), || (), |_, _| crate::el()));
    let handle = app.handle();
    assert!(handle.shared.core.gpu_failure.get().is_none());
    let _app = app.terminal_gpu_loss();
    let latch = handle.shared.core.gpu_failure.get().expect("the same handle sees the builder policy");
    latch.record("device lost".into());
    assert_eq!(handle.window_error().as_deref(), Some("device lost"));
}

#[cfg(all(not(target_arch = "wasm32"), feature = "test-gpu-noop"))]
#[test]
fn worker_observes_loss_without_consuming_host_teardown() {
    let gpu = Gpu::test_noop();
    let latch = Arc::new(Failure::default());
    let mut policy = Policy::Terminal(latch.clone());
    policy.watch_setup(&gpu, || Arc::new(|| {})).unwrap();
    Policy::Recover.watch_setup(&gpu, || panic!("recover must retain the existing waker")).unwrap();
    gpu.device.destroy();
    assert!(gpu.is_lost());
    assert!(!Policy::Recover.failed_on(&gpu));
    assert!(policy.failed_on(&gpu));
    let reason = policy.observe(Some(&gpu)).unwrap_err();
    assert!(reason.contains("GPU device was lost"));
    assert_eq!(latch.message(), Some(reason.clone()));
    assert_eq!(policy.stop("late error".into()), Some(reason));
}
