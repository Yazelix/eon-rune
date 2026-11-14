use super::*;

fn make_overlay() -> Overlay<()> {
    let app = AppSpec::new("failure", (80.0, 40.0), || (), |_, _| crate::el());
    Overlay::new(OverlaySpec::new("failure", app.size, app.view), (), &app).0
}

#[test]
fn terminal_overlay_uses_the_owners_first_reason_and_never_resets_it() {
    let app = crate::App::new(AppSpec::new(
        "owner",
        (80.0, 40.0),
        || 0u8,
        |_, _| crate::el(),
    ));
    let handle = app.handle();
    let app = app.terminal_gpu_loss();
    let latch = app.shared.gpu_failure.get().unwrap().clone();
    let mut overlay = make_overlay();
    overlay.gpu_failure(Some(latch.clone()));
    latch.record("owner failed first".into());
    overlay.ready(false);
    assert!(matches!(overlay.gpu_failure, Policy::Failed));
    assert_eq!(
        overlay.shared.failure.lock().unwrap().as_deref(),
        Some("owner failed first")
    );
    overlay.fail_gpu("later overlay failure".into());
    overlay.ready(true);
    assert_eq!(
        overlay.shared.failure.lock().unwrap().as_deref(),
        Some("owner failed first")
    );
    assert!(!handle.is_visible());
    assert!(Arc::ptr_eq(&latch, app.shared.gpu_failure.get().unwrap()));
    let mut recover = make_overlay();
    recover.gpu_failure(None);
    assert!(!recover.gpu_failed());
}

#[test]
fn terminal_and_failed_overlays_never_use_the_present_worker() {
    assert!(frame::worker_allowed(&Policy::Recover));
    assert!(!frame::worker_allowed(&Policy::Terminal(Arc::default())));
    assert!(!frame::worker_allowed(&Policy::Failed));
}

#[cfg(target_os = "linux")]
#[path = "gpu_failure_native_tests.rs"]
mod native;
