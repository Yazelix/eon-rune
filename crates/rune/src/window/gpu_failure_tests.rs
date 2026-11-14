use super::*;

#[test]
fn terminal_transition_is_permanent_and_reports_only_once() {
    let mut policy = Policy::Terminal;
    assert_eq!(
        policy
            .step("device lost".into())
            .into_iter()
            .collect::<Vec<_>>(),
        ["device lost"]
    );
    assert!(matches!(policy, Policy::Failed));
    assert!(policy.step("later loss".into()).is_empty());
    assert!(matches!(policy, Policy::Failed));
}

#[test]
fn recover_is_the_default_and_never_latches_a_failure() {
    let mut policy = Policy::default();
    assert!(policy.step("validation".into()).is_empty());
    assert!(matches!(policy, Policy::Recover));
    let app = crate::App::new(AppSpec::new(
        "default",
        (30.0, 30.0),
        || (),
        |_, _| crate::el(),
    ));
    assert!(matches!(app.hooks.gpu_failure, Policy::Recover));
    let spec = WindowSpec::from_app("terminal", app.terminal_gpu_loss());
    assert!(matches!(spec.app.hooks.gpu_failure, Policy::Terminal));
}

#[cfg(target_os = "linux")]
#[test]
fn destroyed_device_callback_preserves_default_recovery_and_stops_terminal() {
    let (gpu, _) = super::super::tests::noop_gpu(&super::super::tests::noop_instance());
    assert!(!Policy::Terminal.failed_on(&gpu));
    gpu.device.destroy();
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    assert!(gpu.is_lost());
    assert!(
        gpu.failure
            .message()
            .unwrap()
            .contains("GPU device was lost")
    );
    assert!(Policy::Terminal.failed_on(&gpu));
    assert!(!Policy::Recover.failed_on(&gpu));
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a private headless Wayland session"]
#[allow(
    clippy::disallowed_methods,
    reason = "explicit isolation gate for native test"
)]
fn native_terminal_window_and_same_device_surface_control() {
    assert_eq!(std::env::var("RUNE_GPU_FAILURE_NATIVE").as_deref(), Ok("1"));
    use std::sync::atomic::{AtomicUsize, Ordering};
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    static STARTS: AtomicUsize = AtomicUsize::new(0);
    crate::on_gpu(|event| {
        if event == crate::GpuEvent::Starting {
            STARTS.fetch_add(1, Ordering::SeqCst);
        }
    });
    struct Probe {
        proxy: EventLoopProxy<WakeEvent>,
        phase: Phase,
    }
    enum Phase {
        Setup,
        Shared {
            windows: Windows,
            first: Arc<crate::app::Shared<State>>,
            last: Arc<crate::app::Shared<State>>,
        },
        Done,
    }
    #[derive(Default)]
    struct State {
        inputs: usize,
        polls: usize,
        geometries: usize,
        destroy_on_geometry: Option<wgpu::Device>,
    }
    struct LoseDuringEncode;
    impl Compositor<State> for LoseDuringEncode {
        fn render(&mut self, frame: &mut Frame<'_, State>) {
            frame.device.destroy();
            let _ = frame.device.poll(wgpu::PollType::wait_indefinitely());
        }
    }
    fn host(proxy: EventLoopProxy<WakeEvent>, terminal: bool) -> Host<State> {
        host_at(proxy, terminal, 0, Rc::default())
    }
    fn host_at(
        proxy: EventLoopProxy<WakeEvent>,
        terminal: bool,
        slot: usize,
        pool: Rc<RefCell<Weak<Gpu>>>,
    ) -> Host<State> {
        let mut spec = AppSpec::new(
            "GPU failure probe",
            (200.0, 100.0),
            State::default,
            |_, _| crate::el(),
        );
        spec.poll = Some(|state, _, _| {
            state.polls += 1;
            false
        });
        let app = crate::App::new(spec)
            .on_window_event(|state, _, _, _| {
                state.inputs += 1;
                false
            })
            .on_window_geometry(|state, _, _| {
                state.geometries += 1;
                if let Some(device) = state.destroy_on_geometry.take() {
                    device.destroy();
                    let _ = device.poll(wgpu::PollType::wait_indefinitely());
                }
                false
            });
        let app = if terminal {
            app.terminal_gpu_loss()
        } else {
            app
        };
        Host::new(
            WindowSpec::from_app("probe", app),
            WindowHandle::new("probe"),
            Args::default(),
            proxy,
            slot,
            pool,
            true,
        )
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let mut terminal = host(self.proxy.clone(), true);
            terminal.resume(el);
            assert!(terminal.win.is_some());
            let (gpu, key) = {
                let (gpu, renderer) = terminal.gpu.as_ref().unwrap();
                (gpu.clone(), renderer.device_key())
            };
            let starts = STARTS.load(Ordering::SeqCst);
            terminal.win.as_mut().unwrap().surface_ok = false;
            assert!(terminal.replace_lost_surface());
            assert!(Arc::ptr_eq(&gpu, &terminal.gpu.as_ref().unwrap().0));
            assert_eq!(key, terminal.gpu.as_ref().unwrap().1.device_key());
            assert_eq!(
                STARTS.load(Ordering::SeqCst),
                starts,
                "surface-only recovery creates no device"
            );

            gpu.failure
                .record("injected non-Destroyed failure at the host publisher".into());
            let id = terminal.win.as_ref().unwrap().window.id();
            terminal.handle_window_event(
                el,
                id,
                WindowEvent::CursorEntered {
                    device_id: winit::event::DeviceId::dummy(),
                },
            );
            assert_eq!(
                terminal.rt.state.inputs, 0,
                "failure guard runs before raw input"
            );
            assert!(terminal.win.is_none() && terminal.gpu.is_none());
            let reason = terminal
                .shared
                .window_error
                .lock()
                .unwrap()
                .clone()
                .unwrap();
            assert!(reason.contains("non-Destroyed"));
            terminal.open(el, None);
            terminal.show(el);
            terminal.redraw();
            terminal.first_frame_ready(el);
            terminal.create(el, None, false);
            let samples = terminal.rt.state.geometries;
            assert!(terminal.sample_geometry(el, false));
            assert_eq!(
                terminal.rt.state.geometries, samples,
                "GPU guard precedes geometry notification"
            );
            assert!(terminal.win.is_none() && terminal.gpu.is_none());
            assert!(matches!(terminal.hooks.gpu_failure, Policy::Failed));
            assert_eq!(
                terminal.shared.window_error.lock().unwrap().as_deref(),
                Some(reason.as_str())
            );
            assert_eq!(
                STARTS.load(Ordering::SeqCst),
                starts,
                "reopen/ready/draw never create a replacement"
            );
            let polls = terminal.rt.state.polls;
            terminal.poll_hidden(el, false);
            assert!(
                terminal.rt.state.polls > polls,
                "app polling remains available for failure handling"
            );

            let mut recover = host(self.proxy.clone(), false);
            recover.resume(el);
            let old = recover.gpu.as_ref().unwrap().0.clone();
            let old_key = recover.gpu.as_ref().unwrap().1.device_key();
            old.device.destroy();
            old.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            assert!(!recover.gpu_failed(), "Recover is a negative control");
            assert!(recover.replace_lost_surface());
            assert!(!Arc::ptr_eq(&old, &recover.gpu.as_ref().unwrap().0));
            assert_ne!(old_key, recover.gpu.as_ref().unwrap().1.device_key());

            let mut destroyed = host(self.proxy.clone(), true);
            destroyed.resume(el);
            let gpu = destroyed.gpu.as_ref().unwrap().0.clone();
            let starts = STARTS.load(Ordering::SeqCst);
            gpu.device.destroy();
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            destroyed.redraw();
            assert!(destroyed.win.is_none() && destroyed.gpu.is_none());
            assert_eq!(STARTS.load(Ordering::SeqCst), starts);
            assert!(
                destroyed
                    .shared
                    .window_error
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .contains("GPU device was lost")
            );

            let mut refused = host(self.proxy.clone(), true);
            refused.resume(el);
            let gpu = refused.gpu.as_ref().unwrap().0.clone();
            let starts = STARTS.load(Ordering::SeqCst);
            let window = refused.win.as_mut().unwrap();
            window.config.width = gpu.device.limits().max_texture_dimension_2d + 1;
            window.surface_ok = false;
            assert!(
                !refused.replace_lost_surface(),
                "healthy-device recreation failure is terminal"
            );
            assert!(!gpu.is_lost());
            assert!(refused.win.is_none() && refused.gpu.is_none());
            assert_eq!(
                STARTS.load(Ordering::SeqCst),
                starts,
                "refused configuration cannot request another device"
            );
            let mut waking = host(self.proxy.clone(), true);
            waking.resume(el);
            assert!(waking.rt.wake_deadline().is_none());
            let gpu = waking.gpu.as_ref().unwrap().0.clone();
            assert!(waking.run_hook(el, |_, tx| {
                gpu.failure
                    .record("loss inside wake-requesting hook".into());
                tx.wake_after(Duration::from_secs(12345));
                false
            }));
            assert!(waking.win.is_none());
            assert!(waking.rt.wake_deadline().is_some_and(|due| due >= 12345.0));

            let mut during = host(self.proxy.clone(), true);
            let mut callback = host(self.proxy.clone(), true);
            callback.resume(el);
            callback.rt.state.destroy_on_geometry =
                Some(callback.gpu.as_ref().unwrap().0.device.clone());
            callback.hooks.last_geometry = None;
            let starts = STARTS.load(Ordering::SeqCst);
            assert!(callback.sample_geometry(el, false));
            assert!(callback.win.is_none() && matches!(callback.hooks.gpu_failure, Policy::Failed));
            assert_eq!(
                STARTS.load(Ordering::SeqCst),
                starts,
                "loss during geometry callback stops the host"
            );
            during.comp = Some(Box::new(LoseDuringEncode));
            during.resume(el);
            let starts = STARTS.load(Ordering::SeqCst);
            let presents = during.meter.totals().presents;
            during.redraw();
            assert!(during.win.is_none() && matches!(during.hooks.gpu_failure, Policy::Failed));
            assert_eq!(
                during.meter.totals().presents,
                presents,
                "loss in the compositor cannot publish a frame"
            );
            assert_eq!(STARTS.load(Ordering::SeqCst), starts);

            let mut bounded = host(self.proxy.clone(), true);
            bounded.resume(el);
            let starts = STARTS.load(Ordering::SeqCst);
            for _ in 0..SURFACE_TRIES {
                bounded.win.as_mut().unwrap().surface_ok = false;
                assert!(bounded.replace_lost_surface());
            }
            bounded.win.as_mut().unwrap().surface_ok = false;
            assert!(!bounded.replace_lost_surface());
            assert!(bounded.win.is_none() && matches!(bounded.hooks.gpu_failure, Policy::Failed));
            assert_eq!(STARTS.load(Ordering::SeqCst), starts);
            let pool: Rc<RefCell<Weak<Gpu>>> = Rc::default();
            let mut default = host_at(self.proxy.clone(), false, 0, pool.clone());
            default.resume(el);
            let marked = default.gpu.as_ref().unwrap().0.clone();
            marked
                .failure
                .record("historical uncaptured error on default device".into());
            let mut other_default = host_at(self.proxy.clone(), false, 1, pool.clone());
            other_default.resume(el);
            assert!(Arc::ptr_eq(&marked, &other_default.gpu.as_ref().unwrap().0));
            let mut fresh = host_at(self.proxy.clone(), true, 2, pool);
            fresh.resume(el);
            assert!(
                fresh.win.is_some(),
                "new terminal host skips a marked pooled device"
            );
            assert!(!Arc::ptr_eq(&marked, &fresh.gpu.as_ref().unwrap().0));
            assert!(fresh.shared.window_error.lock().unwrap().is_none());

            // Windows::new_events checks every host before dispatching the targeted wake.
            let pool: Rc<RefCell<Weak<Gpu>>> = Rc::default();
            let mut first = host_at(self.proxy.clone(), true, 0, pool.clone());
            let mut last = host_at(self.proxy.clone(), true, 1, pool.clone());
            first.resume(el);
            last.resume(el);
            let gpu = first.gpu.as_ref().unwrap().0.clone();
            assert!(Arc::ptr_eq(&gpu, &last.gpu.as_ref().unwrap().0));
            let first_handle = first.shared.clone();
            let last_handle = last.shared.clone();
            self.phase = Phase::Shared {
                windows: Windows {
                    factory: None,
                    args: Args::default(),
                    proxy: self.proxy.clone(),
                    hosts: vec![Some(Box::new(first)), Some(Box::new(last))],
                    gpu: pool,
                },
                first: first_handle,
                last: last_handle,
            };
            std::thread::spawn(move || gpu.failure.record("shared idle device failure".into()))
                .join()
                .unwrap();
        }
        fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) {
            let Phase::Shared {
                mut windows,
                first,
                last,
            } = std::mem::replace(&mut self.phase, Phase::Done)
            else {
                return;
            };
            windows.new_events(el, cause);
            for shared in [first, last] {
                assert_eq!(
                    shared.window_error.lock().unwrap().as_deref(),
                    Some("shared idle device failure")
                );
                assert!(!shared.visible.load(Ordering::Acquire));
            }
            let mut quitting = host(self.proxy.clone(), true);
            quitting.resume(el);
            let gpu = quitting.gpu.as_ref().unwrap().0.clone();
            assert!(quitting.run_hook(el, |_, tx| {
                gpu.failure.record("loss inside quitting hook".into());
                tx.quit();
                false
            }));
            assert!(
                el.exiting(),
                "loss must not discard the hook's quit request"
            );
            println!(
                "TERMINAL WINDOW PASS: failure, callback/encode loss, surface identity/refusal, Recover control, bounded retries, shared idle wake, clean pooled start, hook effects"
            );
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event()
        .with_wayland()
        .with_any_thread(true)
        .build()
        .unwrap();
    let proxy = event_loop.create_proxy();
    event_loop
        .run_app(&mut Probe {
            proxy,
            phase: Phase::Setup,
        })
        .unwrap();
}

#[cfg(target_os = "linux")]
#[path = "gpu_failure_regression_tests.rs"]
pub(crate) mod regressions;
