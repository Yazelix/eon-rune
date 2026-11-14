use super::*;
use winit::application::ApplicationHandler;
use winit::platform::wayland::EventLoopBuilderExtWayland;

#[derive(Default)]
struct State {
    fail_in_poll: Option<(Arc<Gpu>, bool)>,
    polls: usize,
}

type Latch = Arc<crate::render::failure::Failure>;

#[allow(
    clippy::disallowed_methods,
    reason = "explicit private-session native test gate"
)]
fn run_case(case: fn(&mut Overlay<State>, &Latch, &ActiveEventLoop)) {
    run_case_with_policy(true, case);
}

static WAKES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[allow(
    clippy::disallowed_methods,
    reason = "explicit private-session native test gate"
)]
fn run_case_with_policy(terminal: bool, case: fn(&mut Overlay<State>, &Latch, &ActiveEventLoop)) {
    assert_eq!(std::env::var("RUNE_GPU_FAILURE_NATIVE").as_deref(), Ok("1"));
    struct Probe {
        overlay: Overlay<State>,
        latch: Latch,
        case: fn(&mut Overlay<State>, &Latch, &ActiveEventLoop),
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            self.overlay.show_fading(el, Duration::ZERO);
            assert!(self.overlay.surface.live().is_some());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (self.case)(&mut self.overlay, &self.latch, el);
            }));
            // Raw layer-shell surfaces must go before the event loop releases its display.
            self.overlay.release();
            if let Err(error) = result {
                std::panic::resume_unwind(error);
            }
            el.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event()
        .with_wayland()
        .with_any_thread(true)
        .build()
        .unwrap();
    let mut spec = AppSpec::new(
        "owned overlay probe",
        (80.0, 40.0),
        State::default,
        |_, _| crate::el(),
    );
    spec.poll = Some(|state, _, tx| {
        state.polls += 1;
        if let Some((gpu, blend)) = state.fail_in_poll.take() {
            let _invalid = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("overlay poll validation"),
                size: 4,
                usage: wgpu::BufferUsages::empty(),
                mapped_at_creation: false,
            });
            tx.set_srgb_blend(blend);
            true
        } else {
            false
        }
    });
    let app = crate::App::new(spec);
    let app = if terminal {
        app.terminal_gpu_loss()
    } else {
        app
    };
    let latch = app.shared.gpu_failure.get().cloned().unwrap_or_default();
    let (mut overlay, _) = Overlay::new(
        OverlaySpec::new("owned overlay probe", app.spec.size, app.spec.view),
        State::default(),
        &app.spec,
    );
    overlay.gpu_failure(app.shared.gpu_failure.get().cloned());
    let proxy = event_loop.create_proxy();
    overlay.connect(
        Arc::new(move || {
            WAKES.fetch_add(1, Ordering::SeqCst);
            let _ = proxy.send_event(WakeEvent::App(0));
        }),
        Arc::new(|| {}),
        Rc::default(),
    );
    event_loop
        .run_app(&mut Probe {
            overlay,
            latch,
            case,
        })
        .unwrap();
}

fn collect_errors(gpu: &Arc<Gpu>) -> Arc<Mutex<Vec<String>>> {
    // Unit builds omit the production uncaptured handler; publish through its real seam.
    let failure = gpu.failure.clone();
    let errors = Arc::new(Mutex::new(Vec::new()));
    let collected = errors.clone();
    gpu.device.on_uncaptured_error(Arc::new(move |error| {
        let message = error.to_string();
        collected.lock().unwrap().push(message.clone());
        failure.record(message);
    }));
    errors
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn overlay_finish_validation_never_submits() {
    struct BadEncoder;
    impl Compositor<State> for BadEncoder {
        fn render(&mut self, frame: &mut Frame<'_, State>) {
            frame.encoder.pop_debug_group();
        }
    }
    run_case(|overlay, latch, _| {
        let gpu = overlay.gpu.clone().unwrap();
        let errors = collect_errors(&gpu);
        overlay.compositor = Some(Box::new(BadEncoder));
        overlay.dirty = true;
        overlay.draw_frame(false);
        let errors = errors.lock().unwrap();
        assert_eq!(
            errors.len(),
            1,
            "finish must not be followed by Queue::submit: {errors:?}"
        );
        assert!(errors[0].contains("Cannot pop debug group"));
        assert!(latch.message().is_some());
        assert!(overlay.surface.live().is_none() && overlay.gpu.is_none());
    });
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn hidden_overlay_loss_never_reopens_on_a_replacement() {
    use std::sync::atomic::AtomicUsize;
    static STARTS: AtomicUsize = AtomicUsize::new(0);
    crate::on_gpu(|event| {
        if event == crate::GpuEvent::Starting {
            STARTS.fetch_add(1, Ordering::SeqCst);
        }
    });
    run_case(|overlay, latch, el| {
        let gpu = overlay.gpu.clone().unwrap();
        overlay.hide();
        assert!(overlay.surface.live().is_none() && overlay.gpu.is_some());
        gpu.device.destroy();
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        assert!(gpu.is_lost());
        let starts = STARTS.load(Ordering::SeqCst);
        overlay.prepare(el, true);
        assert!(
            overlay.surface.live().is_none(),
            "no replacement surface after owned loss"
        );
        assert_eq!(
            STARTS.load(Ordering::SeqCst),
            starts,
            "no replacement device"
        );
        let reason = latch.message().unwrap();
        overlay.show_fading(el, Duration::ZERO);
        overlay.ready(false);
        overlay.prepare(el, true);
        assert_eq!(
            overlay.shared.failure.lock().unwrap().as_deref(),
            Some(reason.as_str()),
            "prepare must never clear Terminal failure"
        );
    });
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn retained_overlay_loss_inside_features_never_replaces_its_device() {
    struct LoseDevice(Arc<Gpu>);
    impl Compositor<State> for LoseDevice {
        fn features(&self) -> wgpu::Features {
            self.0.device.destroy();
            let _ = self.0.device.poll(wgpu::PollType::wait_indefinitely());
            wgpu::Features::empty()
        }
        fn render(&mut self, _: &mut Frame<'_, State>) {}
    }
    run_case(|overlay, latch, el| {
        let gpu = overlay.gpu.clone().unwrap();
        overlay.hide();
        assert!(overlay.surface.live().is_none() && !gpu.is_lost());
        overlay.compositor = Some(Box::new(LoseDevice(gpu.clone())));
        overlay.prepare(el, true);
        assert!(gpu.is_lost(), "features callback must run");
        assert!(
            latch.message().is_some(),
            "retained loss must latch before disposal"
        );
        assert!(overlay.gpu.is_none() && overlay.surface.live().is_none());
        overlay.show_fading(el, Duration::ZERO);
        assert!(overlay.gpu.is_none() && overlay.surface.live().is_none());
    });
}

fn initial_configuration_failure(overlay: &mut Overlay<State>, el: &ActiveEventLoop) -> Arc<Gpu> {
    let gpu = overlay.gpu.clone().unwrap();
    overlay.hide();
    overlay.at.size = (
        (gpu.device.limits().max_texture_dimension_2d + 1) as f32,
        40.0,
    );
    let window = Arc::new(
        el.create_window(Window::default_attributes().with_visible(false))
            .unwrap(),
    );
    assert!(overlay.attach(Backing::Window(window), 1.0).is_none());
    assert!(
        gpu.failure.message().is_none(),
        "configure errors stay inside its error scope"
    );
    assert!(!gpu.is_lost());
    gpu
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn initial_overlay_configuration_failure_latches_the_owner() {
    run_case(|overlay, latch, el| {
        let _gpu = initial_configuration_failure(overlay, el);
        assert!(
            latch.message().is_some(),
            "initial configuration refusal must latch"
        );
        assert!(latch.message().unwrap().contains("could not configure"));
        overlay.at.size = (80.0, 40.0);
        overlay.prepare(el, true);
        overlay.show_fading(el, Duration::ZERO);
        assert!(overlay.gpu.is_none() && overlay.surface.live().is_none());
    });
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn recover_initial_overlay_configuration_failure_stays_local() {
    run_case_with_policy(false, |overlay, _, el| {
        let gpu = initial_configuration_failure(overlay, el);
        assert!(matches!(
            overlay.gpu_failure,
            crate::window::gpu_failure::Policy::Recover
        ));
        assert!(overlay.shared.failure.lock().unwrap().is_some());
        overlay.at.size = (80.0, 40.0);
        overlay.prepare(el, true);
        assert!(overlay.shared.failure.lock().unwrap().is_none());
        assert!(overlay.surface.live().is_some());
        assert!(Arc::ptr_eq(&gpu, overlay.gpu.as_ref().unwrap()));
    });
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn overlay_poll_validation_stops_before_reacquiring() {
    run_case(|overlay, latch, _| {
        overlay.draw_frame(false);
        let gpu = overlay.gpu.clone().unwrap();
        let errors = collect_errors(&gpu);
        let reblends = crate::window::gpu_failure::tests::regressions::REBLENDS.get();
        overlay.rt.state.fail_in_poll = Some((gpu.clone(), !overlay.rt.srgb_blend()));
        overlay.dirty = true;
        overlay.draw_frame(false);
        assert!(overlay.rt.state.fail_in_poll.is_none());
        assert!(!gpu.is_lost());
        assert_eq!(
            crate::window::gpu_failure::tests::regressions::REBLENDS.get(),
            reblends,
            "validation must stop before reblend/reacquire"
        );
        assert_eq!(errors.lock().unwrap().len(), 1);
        assert!(latch.message().is_some() && overlay.surface.live().is_none());
    });
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn overlay_offscreen_loss_latches_owner_and_keeps_updates_pollable() {
    struct LoseDevice;
    impl Compositor<State> for LoseDevice {
        fn render(&mut self, frame: &mut Frame<'_, State>) {
            frame.device.destroy();
            let _ = frame.device.poll(wgpu::PollType::wait_indefinitely());
        }
    }
    run_case(|overlay, latch, el| {
        overlay.compositor = Some(Box::new(LoseDevice));
        overlay.warm_offscreen((80, 40));
        assert!(latch.message().is_some());
        assert!(overlay.surface.live().is_none() && overlay.gpu.is_none());
        let polls = overlay.rt.state.polls;
        overlay.apply_wake(
            true,
            vec![Box::new(|state, _| {
                state.polls += 10;
                true
            })],
        );
        assert!(overlay.rt.state.polls > polls + 10);
        overlay.show_fading(el, Duration::ZERO);
        assert!(overlay.surface.live().is_none());
    });
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn recover_overlay_keeps_its_wake_and_reopen_behavior() {
    run_case_with_policy(false, |overlay, _, el| {
        let gpu = overlay.gpu.clone().unwrap();
        WAKES.store(0, Ordering::SeqCst);
        gpu.failure.record("default device diagnostic".into());
        assert_eq!(
            WAKES.load(Ordering::SeqCst),
            0,
            "Recover must not gain a failure wake subscription"
        );
        assert!(!overlay.gpu_failed());
        overlay.hide();
        overlay.show_fading(el, Duration::ZERO);
        assert!(overlay.surface.live().is_some());
        assert!(Arc::ptr_eq(&gpu, overlay.gpu.as_ref().unwrap()));
        assert!(overlay.shared.failure.lock().unwrap().is_none());
    });
}
