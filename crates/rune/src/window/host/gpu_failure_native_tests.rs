use super::{Host, tests::test_host};
use crate::{App, AppSpec, WindowSpec, window::WakeEvent};
use winit::{application::ApplicationHandler, event::{DeviceId, WindowEvent}, event_loop::{ActiveEventLoop, EventLoop}, platform::wayland::EventLoopBuilderExtWayland, window::WindowId};

#[derive(Default)]
struct Seen { raw: usize, geometry: usize, polls: usize, closed: usize }

fn host(event_loop: &EventLoop<WakeEvent>, terminal: bool) -> Host {
    let mut spec = AppSpec::new("GPU failure probe", (200., 100.), Seen::default, |_, _| crate::el());
    spec.poll = Some(|seen, _, _| { seen.polls += 1; false });
    let app = App::new(spec).keep_running()
        .on_window_event(|seen, _, _, _| { seen.raw += 1; false })
        .on_window_geometry(|seen, _, _| { seen.geometry += 1; false })
        .on_close(|seen, _| { seen.closed += 1; });
    let app = if terminal { app.terminal_gpu_loss() } else { app };
    test_host(WindowSpec::from_app("gpu-failure", app), event_loop, true)
}

#[test]
#[ignore = "requires private headless Wayland and Vulkan"]
#[allow(clippy::disallowed_methods, reason = "native test checks the isolation guard")]
fn native_terminal_window_and_same_device_surface_control() {
    assert_eq!(std::env::var("RUNE_GPU_FAILURE_NATIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("RUNE_CPU_FIRST").as_deref(), Ok("0"));
    assert!(std::env::var_os("DISPLAY").is_none());
    struct Probe { terminal: Host, recover: Host, ran: bool }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, active: &ActiveEventLoop) {
            let host = &mut self.terminal;
            host.resume(active);
            let gpu = host.gpu.as_ref().unwrap().0.clone();
            host.win.as_mut().unwrap().surface_ok = false;
            assert!(host.replace_lost_surface());
            assert!(std::sync::Arc::ptr_eq(&gpu, &host.gpu.as_ref().unwrap().0));

            let id = host.win.as_ref().unwrap().window.id();
            let counts = (host.typed::<Seen>().rt.state.raw, host.typed::<Seen>().rt.state.geometry);
            gpu.failure.record("injected device failure".into());
            host.handle_window_event(active, id, WindowEvent::CursorEntered { device_id: DeviceId::dummy() });
            assert!(host.win.is_none() && host.gpu.is_none());
            assert_eq!(host.shared.window_error().as_deref(), Some("injected device failure"));
            host.open(active, None);
            host.show(active);
            host.draw(false);
            assert!(host.sample_geometry(active, false));
            assert_eq!((host.typed::<Seen>().rt.state.raw, host.typed::<Seen>().rt.state.geometry), counts);
            assert!(host.win.is_none() && host.gpu.is_none());
            let polls = host.typed::<Seen>().rt.state.polls;
            host.poll_hidden(active, false);
            assert!(host.typed::<Seen>().rt.state.polls > polls);
            assert_eq!(host.typed::<Seen>().rt.state.closed, 0);

            let host = &mut self.recover;
            host.resume(active);
            let gpu = host.gpu.as_ref().unwrap().0.clone();
            gpu.device.destroy();
            assert!(!host.gpu_failed());
            assert!(host.replace_lost_surface());
            assert!(!std::sync::Arc::ptr_eq(&gpu, &host.gpu.as_ref().unwrap().0));
            self.ran = true;
            active.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let mut builder = EventLoop::<WakeEvent>::with_user_event();
    builder.with_wayland().with_any_thread(true);
    let event_loop = builder.build().unwrap();
    let mut probe = Probe { terminal: host(&event_loop, true), recover: host(&event_loop, false), ran: false };
    event_loop.run_app(&mut probe).unwrap();
    assert!(probe.ran);
    println!("terminal-gpu-loss-native: pass");
}
