use super::*;
pub(crate) use crate::window::gpu_failure::REBLENDS;
use std::cell::RefCell;
use std::sync::Mutex;
use winit::platform::wayland::EventLoopBuilderExtWayland;

#[derive(Default)]
struct State {
    destroy_in_view: RefCell<Option<wgpu::Device>>,
    fail_in_poll: Option<(Arc<Gpu>, bool)>,
}

type Errors = Arc<Mutex<Vec<String>>>;

fn collect_errors(gpu: &Arc<Gpu>) -> Errors {
    // Unit-test builds omit the production uncaptured handler; use its recording seam.
    let failure = gpu.failure.clone();
    let errors = Errors::default();
    let collected = errors.clone();
    gpu.device.on_uncaptured_error(Arc::new(move |error| {
        let message = error.to_string();
        collected.lock().unwrap().push(message.clone());
        failure.record(message);
    }));
    errors
}

#[allow(
    clippy::disallowed_methods,
    reason = "explicit isolation gate for native test"
)]
fn run_case(fit_height: bool, case: fn(&mut Host<State>, &ActiveEventLoop)) {
    assert_eq!(std::env::var("RUNE_GPU_FAILURE_NATIVE").as_deref(), Ok("1"));
    struct Probe {
        host: Host<State>,
        case: fn(&mut Host<State>, &ActiveEventLoop),
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            self.host.resume(el);
            assert!(self.host.win.is_some());
            (self.case)(&mut self.host, el);
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
        "GPU callback boundary probe",
        (200.0, 100.0),
        State::default,
        |state, _| {
            if let Some(device) = state.destroy_in_view.borrow_mut().take() {
                device.destroy();
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            }
            crate::el().h(100.0)
        },
    );
    spec.poll = Some(|state, _, tx| {
        if let Some((gpu, blend)) = state.fail_in_poll.take() {
            let _invalid = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("poll validation probe"),
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
    let app = crate::App::new(spec).args(Vec::new()).terminal_gpu_loss();
    let app = if fit_height {
        app.fit_content_height()
    } else {
        app
    };
    let host = Host::new(
        WindowSpec::from_app("callback-probe", app),
        WindowHandle::new("callback-probe"),
        Args::default(),
        event_loop.create_proxy(),
        0,
        Rc::default(),
        true,
    );
    event_loop.run_app(&mut Probe { host, case }).unwrap();
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn encoder_finish_validation_never_reaches_queue_submit() {
    struct BadEncoder;
    impl Compositor<State> for BadEncoder {
        fn render(&mut self, frame: &mut Frame<'_, State>) {
            frame.encoder.pop_debug_group();
        }
    }
    run_case(false, |host, _| {
        let gpu = host.gpu.as_ref().unwrap().0.clone();
        let errors = collect_errors(&gpu);
        host.comp = Some(Box::new(BadEncoder));
        host.redraw();
        assert!(host.win.is_none() && matches!(host.hooks.gpu_failure, Policy::Failed));
        let errors = errors.lock().unwrap();
        assert!(errors[0].contains("Cannot pop debug group"), "{errors:?}");
        assert_eq!(
            errors.len(),
            1,
            "invalid commands must never reach Queue::submit: {errors:?}"
        );
        println!("FINISH BOUNDARY PASS");
    });
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn fit_height_reopen_view_loss_never_creates_a_replacement() {
    run_case(true, |host, el| {
        host.redraw();
        let gpu = host.gpu.as_ref().unwrap().0.clone();
        host.hide(el);
        assert!(host.win.is_none() && host.gpu.is_some());
        host.rt
            .update(|state, _| *state.destroy_in_view.borrow_mut() = Some(gpu.device.clone()));
        host.create(el, None, false);
        assert!(gpu.is_lost(), "the reopen view callback must exercise loss");
        assert!(
            matches!(host.hooks.gpu_failure, Policy::Failed),
            "reopen must latch the old device's failure"
        );
        assert!(
            host.win.is_none() && host.gpu.is_none(),
            "no replacement device survives reopen"
        );
        println!("REOPEN BOUNDARY PASS");
    });
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn poll_validation_stops_before_reblend_and_reacquire() {
    run_case(false, |host, _| {
        host.redraw();
        let gpu = host.gpu.as_ref().unwrap().0.clone();
        let errors = collect_errors(&gpu);
        let reblends = REBLENDS.get();
        host.rt.state.fail_in_poll = Some((gpu.clone(), !host.rt.srgb_blend()));
        host.redraw();
        assert!(!gpu.is_lost(), "validation must leave the device alive");
        assert!(
            host.rt.state.fail_in_poll.is_none(),
            "poll must run inside the acquired frame"
        );
        assert!(host.win.is_none() && matches!(host.hooks.gpu_failure, Policy::Failed));
        assert_eq!(
            REBLENDS.get(),
            reblends,
            "known failure must stop before reblend/reacquisition"
        );
        assert_eq!(errors.lock().unwrap().len(), 1);
        println!("POLL BOUNDARY PASS");
    });
}

#[test]
#[ignore = "requires a private headless Wayland session"]
fn guarded_video_submit_preserves_notifications_and_rejects_invalid_commands() {
    run_case(false, |host, _| {
        let gpu = host.gpu.as_ref().unwrap().0.clone();
        let errors = collect_errors(&gpu);
        let surface = crate::native_surface::NativeSurface::new();
        surface
            .push_bytes(crate::native_surface::synthetic(16, 16, 0), 1)
            .unwrap();
        let (send, recv) = std::sync::mpsc::channel();
        surface.set_observer(Arc::new(move |timing| {
            send.send(timing).unwrap();
        }));
        let mut renderer = crate::native_surface::SurfaceRenderer::new(
            &gpu.device,
            crate::render::OFFSCREEN_FORMAT,
        );
        renderer.prepare(
            &gpu.device,
            &gpu.queue,
            &[crate::native_surface::SurfaceDraw {
                surface: surface.clone(),
                fit: crate::native_surface::SurfaceFit::Fit,
                before_instance: 0,
                rect: [0.0, 0.0, 16.0, 16.0],
                radii: [0.0; 4],
                clip: crate::render::NO_CLIP,
                fade: [0; 4],
                opacity: 1.0,
            }],
            (16, 16),
        );
        let target = crate::render::Offscreen::new(&gpu.device, 16, 16);
        let encode = || {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    timestamp_writes: renderer.timestamp_writes(),
                    ..Default::default()
                });
                renderer.draw(&mut pass, 0);
            }
            encoder
        };
        assert!(renderer.submit_if(&gpu.queue, encode(), || false).is_none());
        assert!(
            recv.try_recv().is_err(),
            "rejected work has no submission notice"
        );
        // The public default path still delivers submission and completion for video.
        renderer.submit(&gpu.queue, encode());
        assert!(
            recv.recv_timeout(Duration::from_secs(2))
                .unwrap()
                .completed_at
                .is_none()
        );
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        assert!(
            recv.recv_timeout(Duration::from_secs(2))
                .unwrap()
                .completed_at
                .is_some()
        );
        let failure = gpu.failure.clone();
        surface.set_observer(Arc::new(move |_| {
            failure.record("failure inside video observer".into())
        }));
        assert!(
            renderer
                .submit_if(&gpu.queue, encode(), || gpu.failure.message().is_none())
                .is_none(),
            "observer failure must stop the following queue submission"
        );
        let mut invalid = encode();
        invalid.pop_debug_group();
        assert!(
            renderer
                .submit_if(&gpu.queue, invalid, || gpu.failure.message().is_none())
                .is_none()
        );
        assert_eq!(
            errors.lock().unwrap().len(),
            1,
            "finish failure never submits"
        );
        assert!(
            recv.try_recv().is_err(),
            "failed video work has no submission notice"
        );
        println!("VIDEO SUBMISSION BOUNDARY PASS");
    });
}
