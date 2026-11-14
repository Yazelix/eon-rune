use super::*;
use crate::overlay::{Availability, OverlaySpec};
use winit::platform::wayland::EventLoopBuilderExtWayland;

#[test]
#[ignore = "requires a private headless Wayland session"]
#[allow(
    clippy::disallowed_methods,
    reason = "explicit private-session native test gate"
)]
fn lost_ready_owned_warm_never_creates_a_replacement_device() {
    use std::sync::atomic::AtomicUsize;
    static STARTS: AtomicUsize = AtomicUsize::new(0);
    crate::on_gpu(|event| {
        if event == crate::GpuEvent::Starting {
            STARTS.fetch_add(1, Ordering::SeqCst);
        }
    });
    struct Probe(winit::event_loop::EventLoopProxy<WakeEvent>);
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let app = crate::App::new(AppSpec::new(
                "ready warm owner",
                (80.0, 40.0),
                || (),
                |_, _| crate::el(),
            ))
            .args(Vec::new())
            .terminal_gpu_loss();
            let handle = app.handle();
            let latch = app.shared.gpu_failure.get().unwrap().clone();
            let (send, recv) = std::sync::mpsc::channel();
            handle.warm_gpu_with(wgpu::Features::empty(), move |device| {
                send.send(device.clone()).unwrap();
            });
            let device = recv.recv_timeout(Duration::from_secs(10)).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !Gpu::warm_ready_for_test() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "worker must publish Ready"
                );
                std::thread::yield_now();
            }
            assert!(latch.message().is_none());
            device.destroy();
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let starts = STARTS.load(Ordering::SeqCst);
            let mut host = Host::new(
                WindowSpec::from_app("ready-warm", app),
                WindowHandle::new("ready-warm"),
                Args::default(),
                self.0.clone(),
                0,
                Rc::default(),
                true,
            );
            host.resume(el);
            assert_eq!(
                STARTS.load(Ordering::SeqCst),
                starts,
                "lost Ready warm must not create a replacement device"
            );
            assert!(latch.message().is_some());
            assert!(host.win.is_none() && host.gpu.is_none());
            assert_eq!(handle.window_error(), latch.message());
            el.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    assert_eq!(std::env::var("RUNE_GPU_FAILURE_NATIVE").as_deref(), Ok("1"));
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event()
        .with_wayland()
        .with_any_thread(true)
        .build()
        .unwrap();
    let proxy = event_loop.create_proxy();
    event_loop.run_app(&mut Probe(proxy)).unwrap();
}

#[test]
#[ignore = "requires a private headless Wayland session"]
#[allow(
    clippy::disallowed_methods,
    reason = "explicit private-session native test gate"
)]
fn failed_cold_warm_never_falls_back_to_a_replacement_device() {
    use std::sync::atomic::AtomicUsize;
    static STARTS: AtomicUsize = AtomicUsize::new(0);
    crate::on_gpu(|event| {
        if event == crate::GpuEvent::Starting {
            STARTS.fetch_add(1, Ordering::SeqCst);
        }
    });
    struct Gate(RefCell<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>);
    impl crate::Compositor<()> for Gate {
        fn features(&self) -> wgpu::Features {
            if let Some((release, failed)) = self.0.borrow_mut().take() {
                release.send(()).unwrap();
                failed.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            wgpu::Features::empty()
        }
        fn render(&mut self, _: &mut crate::Frame<'_, ()>) {}
    }
    struct Probe(winit::event_loop::EventLoopProxy<WakeEvent>);
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let (entered_tx, entered) = std::sync::mpsc::channel();
            let (release, released) = std::sync::mpsc::channel();
            let (failed_tx, failed) = std::sync::mpsc::channel();
            let app = crate::App::new(AppSpec::new(
                "cold warm owner",
                (80.0, 40.0),
                || (),
                |_, _| crate::el(),
            ))
            .args(Vec::new())
            .terminal_gpu_loss()
            .compositor(Gate(RefCell::new(Some((release, failed)))));
            let handle = app.handle();
            let latch = app.shared.gpu_failure.get().unwrap().clone();
            latch
                .watch(Arc::new(move || {
                    failed_tx.send(()).unwrap();
                }))
                .unwrap();
            handle.warm_gpu_with(wgpu::Features::empty(), move |device| {
                entered_tx.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(3)).unwrap();
                device.destroy();
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
            });
            entered.recv_timeout(Duration::from_secs(10)).unwrap();
            let starts = STARTS.load(Ordering::SeqCst);
            let mut host = Host::new(
                WindowSpec::from_app("cold-warm", app),
                WindowHandle::new("cold-warm"),
                Args::default(),
                self.0.clone(),
                0,
                Rc::default(),
                true,
            );
            host.resume(el);
            assert!(latch.message().is_some());
            assert_eq!(
                STARTS.load(Ordering::SeqCst),
                starts,
                "a failed warm slot must not create a replacement device"
            );
            assert!(host.win.is_none() && host.gpu.is_none());
            assert_eq!(handle.window_error(), latch.message());
            el.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    assert_eq!(std::env::var("RUNE_GPU_FAILURE_NATIVE").as_deref(), Ok("1"));
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event()
        .with_wayland()
        .with_any_thread(true)
        .build()
        .unwrap();
    let proxy = event_loop.create_proxy();
    event_loop.run_app(&mut Probe(proxy)).unwrap();
}

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
