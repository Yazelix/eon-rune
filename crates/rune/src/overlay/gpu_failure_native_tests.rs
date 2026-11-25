use super::super::{Backing, Overlay, OverlaySpec};
use crate::render::failure::Failure;
use crate::window::gpu_failure::Policy;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::wayland::EventLoopBuilderExtWayland;
use winit::window::{Window, WindowId};

#[test]
#[ignore = "requires a private Wayland compositor"]
fn terminal_overlay_unpark_releases_its_window() {
    assert_eq!(crate::env::var("RUNE_GPU_FAILURE_NATIVE").as_deref(), Some("1"));
    assert!(crate::env::var_os("DISPLAY").is_none());

    struct Probe(bool);
    impl ApplicationHandler for Probe {
        fn resumed(&mut self, active: &ActiveEventLoop) {
            let attributes = Window::default_attributes().with_visible(false);
            let window = Arc::new(active.create_window(attributes).unwrap());
            let weak = Arc::downgrade(&window);
            let view = |_: &(), _: &mut crate::Cx<()>| crate::el().size(30.0, 30.0);
            let spec = crate::AppSpec::new("terminal-unpark", (30.0, 30.0), || (), view);
            let configured = OverlaySpec::new("terminal-unpark", spec.size, view).options(Default::default());
            let (mut overlay, _) = Overlay::new(configured, (), &spec);
            let failure = Arc::new(Failure::default());
            overlay.gpu_failure = Policy::Terminal(failure.clone());
            overlay.surface.park(Backing::Window(window));
            failure.record("device lost during unpark".into());

            assert!(!overlay.unpark_in_test());
            assert!(matches!(overlay.gpu_failure, Policy::Failed));
            assert!(overlay.surface.backing().is_none(), "terminal attach must release the parked backing");
            assert!(weak.upgrade().is_none(), "the overlay must relinquish its window");
            self.0 = true;
            active.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let mut builder = EventLoop::builder();
    builder.with_wayland().with_any_thread(true);
    let mut probe = Probe(false);
    builder.build().unwrap().run_app(&mut probe).unwrap();
    assert!(probe.0);
    println!("terminal-overlay-unpark: pass");
}
