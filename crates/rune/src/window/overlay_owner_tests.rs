use super::*;
use crate::overlay::{Availability, OverlaySpec};
use winit::platform::wayland::EventLoopBuilderExtWayland;

#[test]
#[ignore = "requires a private headless Wayland session"]
#[allow(
    clippy::disallowed_methods,
    reason = "explicit private-session native test gate"
)]
fn native_start_transfers_the_app_latch_across_state_types() {
    assert_eq!(std::env::var("RUNE_GPU_FAILURE_NATIVE").as_deref(), Ok("1"));
    struct Probe {
        proxy: winit::event_loop::EventLoopProxy<WakeEvent>,
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let mut app = crate::App::new(AppSpec::new(
                "owner",
                (80.0, 40.0),
                || 0u8,
                |_, _| crate::el(),
            ))
            .args(Vec::new())
            .terminal_gpu_loss();
            let handle = app.handle();
            let overlay = app.overlay(
                OverlaySpec::new("child", (80.0, 40.0), |_: &(), _| crate::el()),
                (),
            );
            let mut pending = Vec::new();
            let pool: Rc<RefCell<Weak<Gpu>>> = Rc::default();
            let mut host = crate::window::start(
                crate::WindowSpec::from_app("owner", app),
                None,
                true,
                crate::tooling::Args::default(),
                crate::window::Start {
                    proxy: self.proxy.clone(),
                    slot: 0,
                    gpu: pool.clone(),
                    wayland: true,
                    traced: false,
                    overlays: &mut pending,
                },
            )
            .unwrap();
            host.resumed(el);
            let child = &mut pending[0];
            child.connect(Arc::new(|| {}), Arc::new(|| {}), pool.clone());
            overlay.show();
            child.resumed(el);
            assert_eq!(overlay.availability(), Availability::Available);
            let gpu = pool.borrow().upgrade().unwrap();
            gpu.failure
                .record("overlay observed shared device failure".into());
            child.about_to_wait(el);
            host.new_events(el, StartCause::Poll);
            assert_eq!(overlay.availability(), Availability::Unavailable);
            assert_eq!(overlay.failure(), handle.window_error());
            assert!(!host.reopen_candidate().visible);
            host.about_to_wait(el);
            assert!(!handle.is_visible());
            // Once the owner has failed, a later child's show cannot clear its reason.
            overlay.show();
            child.user_event(el, WakeEvent::App(1));
            assert_eq!(overlay.availability(), Availability::Unavailable);
            el.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event()
        .with_wayland()
        .with_any_thread(true)
        .build()
        .unwrap();
    let proxy = event_loop.create_proxy();
    event_loop.run_app(&mut Probe { proxy }).unwrap();
}
