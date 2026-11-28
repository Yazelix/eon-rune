use crate::{AppSpec, WindowSpec};

#[cfg(target_os = "linux")]
use super::{ActiveEventLoop, Host, WindowEvent, tests::test_host};
#[cfg(target_os = "linux")]
use crate::{Tx, WindowGeometry, WindowHandle, time::Instant, tooling::Args,
    window::{AnyHost, WakeEvent, WindowRequest, Windows, surface::Redraw}};
#[cfg(target_os = "linux")]
use std::{sync::{Arc, Mutex}, time::Duration};
#[cfg(target_os = "linux")]
use winit::{application::ApplicationHandler, dpi::PhysicalPosition, event::StartCause, event_loop::ControlFlow, window::WindowId};

#[test]
fn geometry_hook_is_opt_in_and_survives_window_spec() {
    let app = crate::App::new(AppSpec::new("geometry", (20.0, 20.0), || (), |_, _| crate::el()));
    assert!(app.window_hooks.geometry.is_none() && app.window_hooks.last_geometry.is_none());
    let spec = WindowSpec::from_app("second", app.on_window_geometry(|_, _, _| false));
    assert!(spec.app.window_hooks.geometry.is_some() && spec.app.window_hooks.last_geometry.is_none());
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct Seen {
    snapshots: Vec<WindowGeometry>,
    order: Vec<&'static str>,
    invalidated: bool,
    stale_inputs: usize,
    zero_events: usize,
    pointer_events: usize,
    scale_events: usize,
    receipts: Vec<&'static str>,
    close: bool,
}

#[cfg(target_os = "linux")]
fn observe(s: &mut Seen, geometry: WindowGeometry, tx: &mut Tx) -> bool {
    s.snapshots.push(geometry);
    s.order.push("geometry");
    if s.close {
        tx.close_window();
    }
    false
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires isolated Wayland"]
#[allow(clippy::disallowed_methods, reason = "native test checks its isolation guard")]
fn native_geometry_creation_idle_and_event_order() {
    assert_eq!(std::env::var("RUNE_GEOMETRY_TEST").as_deref(), Ok("1"));
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    enum Phase {
        Admission,
        Resize,
    }
    struct Probe {
        phase: Phase,
        host: Host,
        closing: Host,
        deadline: std::time::Instant,
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            self.closing.resume(el);
            assert!(self.closing.life.finished(), "creation callback close is applied before continuing");
            assert_eq!(self.closing.typed::<Seen>().rt.state.order, ["geometry"], "closed creation does not deliver focus");
            let h = &mut self.host;
            h.resume(el);
            assert_eq!(h.typed::<Seen>().rt.state.snapshots.len(), 1, "creation notifies once");
            assert!(!h.typed::<Seen>().rt.state.snapshots[0].has_output, "native output entry must occur after creation in this fixture");
            assert_eq!(h.typed::<Seen>().rt.state.order.first(), Some(&"geometry"), "geometry precedes initial focus");
            let actual = h.typed::<Seen>().rt.state.snapshots[0];
            let window = &h.win.as_ref().unwrap().window;
            let size = window.inner_size();
            assert_eq!(actual.physical_size, (size.width, size.height));
            assert_eq!(actual.scale_factor, window.scale_factor());
            assert_eq!(actual.has_output, window.current_monitor().is_some());
            let redraw = h.win.as_ref().unwrap().redraw.replace(Redraw::Idle);
            h.sample_geometry(el, false);
            assert_eq!(h.typed::<Seen>().rt.state.snapshots.len(), 1, "unchanged idle is silent");
            assert!(matches!(h.win.as_ref().unwrap().redraw.get(), Redraw::Idle), "unchanged idle asks for no frame");
            h.win.as_ref().unwrap().redraw.set(redraw);
            let cached = h.typed::<Seen>().window_hooks.last_geometry;
            let hook = h.typed_mut::<Seen>().window_hooks.geometry.take();
            assert!(!h.sample_geometry(el, true), "absent hook performs no update");
            assert_eq!(h.typed::<Seen>().window_hooks.last_geometry, cached);
            h.typed_mut::<Seen>().window_hooks.geometry = hook;
            // Inject a different cached output fact; the actual native sample is unchanged.
            h.typed_mut::<Seen>().window_hooks.last_geometry = Some(WindowGeometry { has_output: !actual.has_output, ..actual });
            h.sample_geometry(el, false);
            assert_eq!(h.typed::<Seen>().rt.state.snapshots.len(), 2, "output alone changes the sample");
            assert_eq!(h.typed::<Seen>().rt.state.snapshots[1], actual);
            h.sample_geometry(el, true);
            assert_eq!(h.typed::<Seen>().rt.state.snapshots.len(), 3, "new lifetime forces the equal initial sample");
            let id = h.win.as_ref().unwrap().window.id();
            for size in [(0, 300), (300, 0)] {
                h.typed_mut::<Seen>().rt.state.invalidated = false;
                let before = (h.typed::<Seen>().rt.state.zero_events, h.typed::<Seen>().rt.state.pointer_events, h.typed::<Seen>().rt.state.receipts.len());
                h.handle_window_event(el, id, WindowEvent::Resized(winit::dpi::PhysicalSize::new(size.0, size.1)));
                h.handle_window_event(
                    el,
                    id,
                    WindowEvent::CursorMoved { device_id: winit::event::DeviceId::dummy(), position: PhysicalPosition::new(2.0, 3.0) },
                );
                assert_eq!(h.typed::<Seen>().rt.state.stale_inputs, 0, "each zero-size case invalidates before input");
                assert_eq!((h.typed::<Seen>().rt.state.zero_events, h.typed::<Seen>().rt.state.pointer_events), (before.0 + 1, before.1 + 1), "exact zero-size/input receipt counts");
                assert_eq!(&h.typed::<Seen>().rt.state.receipts[before.2..], &["zero", "input"], "exact zero-size/input receipt order");
            }
            println!("GEOMETRY READY initial={actual:?}");
        }
        fn user_event(&mut self, el: &ActiveEventLoop, event: WakeEvent) {
            self.host.handle_user_event(el, event);
        }
        fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            let scale = matches!(&event, WindowEvent::ScaleFactorChanged { .. }) && self.host.win.as_ref().is_some_and(|w| w.window.id() == id);
            let before = (self.host.typed::<Seen>().rt.state.scale_events, self.host.typed::<Seen>().rt.state.pointer_events, self.host.typed::<Seen>().rt.state.receipts.len());
            if scale {
                self.host.typed_mut::<Seen>().rt.state.invalidated = false;
            }
            self.host.handle_window_event(el, id, event);
            if scale {
                self.host.handle_window_event(
                    el,
                    id,
                    WindowEvent::CursorMoved { device_id: winit::event::DeviceId::dummy(), position: PhysicalPosition::new(4.0, 5.0) },
                );
                assert_eq!(self.host.typed::<Seen>().rt.state.stale_inputs, 0, "native scale invalidates before input");
                assert_eq!((self.host.typed::<Seen>().rt.state.scale_events, self.host.typed::<Seen>().rt.state.pointer_events), (before.0 + 1, before.1 + 1));
                assert_eq!(&self.host.typed::<Seen>().rt.state.receipts[before.2..], &["scale", "input"], "native scale/input receipt order");
            }
        }
        fn about_to_wait(&mut self, el: &ActiveEventLoop) {
            let h = &mut self.host;
            h.about_to_wait(el);
            let g = *h.typed::<Seen>().rt.state.snapshots.last().unwrap();
            let expected = std::env::var("RUNE_GEOMETRY_SCALE").unwrap().parse::<f64>().unwrap();
            let target = ((280.0 * expected) as u32, (180.0 * expected) as u32);
            if matches!(self.phase, Phase::Admission) && g.has_output && h.native_handle.is_ready() {
                assert!(h.typed::<Seen>().rt.state.snapshots[3..].iter().any(|sample| sample.has_output), "idle must observe real output entry");
                assert_ne!(g.physical_size, target, "resize witness must differ from the initial native size");
                let before = h.typed::<Seen>().rt.state.snapshots.len();
                h.native_handle.resize_instant((280.0, 180.0));
                assert_eq!(h.typed::<Seen>().rt.state.snapshots.len(), before, "a queued request is not applied geometry");
                self.phase = Phase::Resize;
            }
            if matches!(self.phase, Phase::Resize) && g.has_output && g.scale_factor == expected && g.physical_size == target {
                let count = h.typed::<Seen>().rt.state.snapshots.len();
                h.sample_geometry(el, false);
                assert_eq!(h.typed::<Seen>().rt.state.snapshots.len(), count);
                if expected != 1.0 {
                    assert!(h.typed::<Seen>().rt.state.scale_events > 0, "fractional native scale event must be witnessed");
                }
                println!("GEOMETRY PASS final={g:?}");
                el.exit();
                return;
            }
            assert!(std::time::Instant::now() < self.deadline, "native geometry did not arrive: {g:?}");
            el.set_control_flow(ControlFlow::WaitUntil(std::time::Instant::now() + Duration::from_millis(20)));
        }
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event().with_wayland().with_any_thread(true).build().unwrap();
    let make = |name, app| test_host(WindowSpec::from_app(name, app), &event_loop, true);
    let app = crate::App::new(AppSpec::new("Native geometry", (300.0, 200.0), Seen::default, |_, _| crate::el()))
        .float_on_open()
        .window_options(crate::WindowOptions { min_size: Some((0.0, 0.0)), ..Default::default() })
        .on_window_geometry(observe)
        .on_focus(|s, _, _| s.order.push("focus"))
        .on_window_event(|s, _, event, _| {
            match event {
                WindowEvent::Resized(size) if size.width == 0 || size.height == 0 => {
                    s.invalidated = true;
                    s.zero_events += 1;
                    s.receipts.push("zero");
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    s.invalidated = true;
                    s.scale_events += 1;
                    s.receipts.push("scale");
                }
                WindowEvent::CursorMoved { .. } => {
                    s.stale_inputs += usize::from(!s.invalidated);
                    s.pointer_events += 1;
                    s.receipts.push("input");
                }
                _ => {}
            }
            true
        });
    let host = make("geometry", app);
    let closing = make(
        "closing",
        crate::App::new(AppSpec::new("Closing geometry", (100.0, 100.0), || Seen { close: true, ..Default::default() }, |_, _| crate::el()))
            .on_window_geometry(observe)
            .on_focus(|s, _, _| s.order.push("focus")),
    );
    event_loop.run_app(&mut Probe { phase: Phase::Admission, host, closing, deadline: std::time::Instant::now() + Duration::from_secs(10) }).unwrap();
}

#[cfg(all(target_os = "linux", rune_x11))]
#[test]
#[ignore = "requires isolated X11"]
#[allow(clippy::disallowed_methods, reason = "native test checks its isolation guard")]
fn native_geometry_hidden_creation_keeps_applied_viewport() {
    assert_eq!(std::env::var("RUNE_GEOMETRY_X11_TEST").as_deref(), Ok("1"));
    use winit::platform::x11::EventLoopBuilderExtX11;
    struct Probe {
        host: Host,
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let h = &mut self.host;
            h.resume(el);
            assert!(!h.life.shown() && !h.life.finished() && h.win.is_some(), "hidden creation retains its native window");
            let window = &h.win.as_ref().unwrap().window;
            let size = window.inner_size();
            assert_eq!((size.width, size.height), (620, 360), "the native resize in on_window was applied");
            let scale = window.scale_factor() as f32;
            let applied = (size.width as f32 / scale, size.height as f32 / scale);
            assert_eq!(h.app.rt().window_viewport(), applied, "hidden creation synchronizes current native viewport");
            h.show(el, crate::time::now());
            assert!(h.life.shown());
            assert_eq!(h.app.rt().window_viewport(), applied, "a later show retains applied geometry");
            println!("HIDDEN CREATION PASS viewport={applied:?}");
            el.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event().with_x11().with_any_thread(true).build().unwrap();
    let app = crate::App::new(AppSpec::new("Hidden geometry", (300.0, 200.0), || Seen { close: true, ..Default::default() }, |_, _| crate::el()))
        .keep_running()
        .window_options(crate::WindowOptions { min_size: Some((0.0, 0.0)), ..Default::default() })
        .on_window(|handle| {
            let _applied = handle.window().unwrap().request_inner_size(winit::dpi::PhysicalSize::new(620, 360));
        })
        .on_window_geometry(observe);
    let host = test_host(WindowSpec::from_app("hidden", app), &event_loop, false);
    event_loop.run_app(&mut Probe { host }).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires isolated Wayland"]
#[allow(clippy::disallowed_methods, reason = "native test checks its isolation guard")]
fn native_bound_focus_close_dispatch() {
    assert_eq!(std::env::var("RUNE_GEOMETRY_TEST").as_deref(), Ok("1"));
    crate::orderly_exit();
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    struct Probe {
        windows: Windows,
        handle: Arc<Mutex<Option<WindowHandle>>>,
        trace: Arc<Mutex<Vec<&'static str>>>,
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) {
            self.windows.new_events(el, cause);
        }
        fn resumed(&mut self, el: &ActiveEventLoop) {
            self.windows.resumed(el);
            let handle = self.handle.lock().unwrap().clone().unwrap();
            handle.request_attention();
            handle.focus();
            handle.close();
        }
        fn user_event(&mut self, el: &ActiveEventLoop, event: WakeEvent) {
            let kind = match &event {
                WakeEvent::Attention(..) => Some("attention-complete"),
                WakeEvent::Focus(..) => Some("focus-complete"),
                WakeEvent::Window(_, WindowRequest::Close, _) => Some("close-complete"),
                _ => None,
            };
            self.windows.user_event(el, event);
            if let Some(kind) = kind {
                self.trace.lock().unwrap().push(kind);
            }
        }
        fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            self.windows.window_event(el, id, event);
        }
        fn about_to_wait(&mut self, el: &ActiveEventLoop) {
            self.windows.about_to_wait(el);
        }
        fn exiting(&mut self, el: &ActiveEventLoop) {
            self.windows.exiting(el);
            self.trace.lock().unwrap().push("loop-exit");
        }
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event().with_wayland().with_any_thread(true).build().unwrap();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let state = trace.clone();
    let handle = Arc::new(Mutex::new(None));
    let created = handle.clone();
    let app = crate::App::new(AppSpec::new("Bound FIFO", (300.0, 200.0), || Arc::new(Mutex::new(Vec::<&'static str>::new())), |_, _| crate::el()))
        .attention_wait(std::time::Duration::from_secs(5))
        .init(move || state)
        .on_close(|s, _| s.lock().unwrap().push("close-hook"));
    let windows = Windows {
        factory: Some(Box::new(move |_| vec![WindowSpec::from_app("fifo", app).on_created(move |h| *created.lock().unwrap() = Some(h)).boxed()])),
        args: Args::default(),
        proxy: event_loop.create_proxy(),
        hosts: Vec::new(),
        gpu: std::rc::Rc::default(),
        any_shown: false,
        startup_error: None,
        display_error: None,
        wayland_wake: Default::default(),
        on_display_close: Vec::new(),
    };
    let mut probe = Probe { windows, handle, trace: trace.clone() };
    event_loop.run_app(&mut probe).unwrap();
    assert!(probe.windows.hosts.iter().all(Option::is_none), "finished native host was cleaned up");
    assert_eq!(*trace.lock().unwrap(), ["attention-complete", "focus-complete", "close-hook", "close-complete", "loop-exit"], "bound native dispatch, close and loop-exit order");
    println!("BOUND FIFO PASS");
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires isolated Wayland"]
#[allow(clippy::disallowed_methods, reason = "native test checks its isolation guard")]
fn native_geometry_idle_close_exits_without_wakes() {
    assert_eq!(std::env::var("RUNE_GEOMETRY_TEST").as_deref(), Ok("1"));
    crate::orderly_exit();
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    struct Probe {
        windows: Windows,
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) {
            self.windows.new_events(el, cause);
        }
        fn resumed(&mut self, el: &ActiveEventLoop) {
            self.windows.resumed(el);
        }
        fn user_event(&mut self, el: &ActiveEventLoop, event: WakeEvent) {
            self.windows.user_event(el, event);
        }
        fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            self.windows.window_event(el, id, event);
        }
        fn about_to_wait(&mut self, el: &ActiveEventLoop) {
            self.windows.about_to_wait(el);
            assert!(!self.windows.hosts.iter().flatten().any(|h| h.finished()), "idle close cleaned before sleeping");
        }
        fn exiting(&mut self, el: &ActiveEventLoop) {
            self.windows.exiting(el);
        }
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event().with_wayland().with_any_thread(true).build().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = seen.clone();
    let app = crate::App::new(AppSpec::new("Idle close", (300.0, 200.0), || Arc::new(Mutex::new(Vec::<WindowGeometry>::new())), |_, _| crate::el()))
        .init(move || state)
        .on_window_geometry(|s, g, tx| {
            let mut seen = s.lock().unwrap();
            let idle = !seen.is_empty();
            if idle {
                assert_ne!(seen[0], g, "native geometry changed on idle");
            }
            seen.push(g);
            if idle {
                println!("IDLE CLOSE REQUEST");
                tx.close_window();
            }
            false
        });
    let windows = Windows {
        factory: Some(Box::new(move |_| vec![WindowSpec::from_app("idle-close", app).boxed()])),
        args: Args::default(),
        proxy: event_loop.create_proxy(),
        hosts: Vec::new(),
        gpu: std::rc::Rc::default(),
        any_shown: false,
        startup_error: None,
        display_error: None,
        wayland_wake: Default::default(),
        on_display_close: Vec::new(),
    };
    let mut probe = Probe { windows };
    event_loop.run_app(&mut probe).unwrap();
    assert!(probe.windows.hosts.iter().all(Option::is_none));
    assert_eq!(seen.lock().unwrap().len(), 2, "creation then changed idle sample");
    println!("IDLE CLOSE PASS");
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires isolated Wayland"]
#[allow(clippy::disallowed_methods, reason = "native test checks its isolation guard")]
fn native_shared_creation_forces_geometry_before_draw_and_recreation() {
    assert_eq!(std::env::var("RUNE_GEOMETRY_TEST").as_deref(), Ok("1"));
    crate::orderly_exit();
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    #[derive(Default)]
    enum Admission { #[default] Allow, Close }
    #[derive(Default)]
    struct Created {
        geometry: Vec<WindowGeometry>,
        order: Vec<&'static str>,
        admission: Admission,
    }
    enum Phase { Waiting(Instant), Done }
    struct Probe {
        host: Host,
        seen: Arc<Mutex<Created>>,
        phase: Phase,
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let h = &mut self.host;
            h.resume(el);
            assert_eq!(self.seen.lock().unwrap().order, ["created", "window", "geometry"]);
            let actual = self.seen.lock().unwrap().geometry[0];
            // Exercise the shared CPU-compatible finish with neither GPU nor surface.
            let gpu = h.gpu.take();
            let surface = h.win.as_mut().unwrap().surface.take();
            h.win.as_mut().unwrap().config.width = 1;
            h.win.as_mut().unwrap().config.height = 1;
            h.app.rt_mut().set_viewport((1.0, 1.0), 1.0);
            assert!(!h.finish_creation(el));
            assert!(h.gpu.is_none() && h.win.as_ref().unwrap().surface.is_none());
            let scale = actual.scale_factor as f32;
            assert_eq!(h.app.rt().window_viewport(), (actual.physical_size.0 as f32 / scale, actual.physical_size.1 as f32 / scale));
            assert_eq!((h.win.as_ref().unwrap().config.width, h.win.as_ref().unwrap().config.height), actual.physical_size);
            assert_eq!(self.seen.lock().unwrap().geometry, [actual, actual], "equal cached geometry still gets an initial callback");
            h.win.as_mut().unwrap().surface = surface;
            h.gpu = gpu;
            h.hide(el);
            assert!(h.win.is_none(), "Wayland recreation must allocate a new native window");
            h.try_create(el, None, false).unwrap();
            assert_eq!(self.seen.lock().unwrap().geometry, [actual, actual, actual], "equal native recreation forces its initial geometry");
        }
        fn user_event(&mut self, el: &ActiveEventLoop, event: WakeEvent) {
            self.host.handle_user_event(el, event);
        }
        fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            self.host.handle_window_event(el, id, event);
        }
        fn about_to_wait(&mut self, el: &ActiveEventLoop) {
            self.host.about_to_wait(el);
            let Phase::Waiting(deadline) = self.phase else { return };
            let ready = self.seen.lock().unwrap().order.contains(&"ready");
            assert!(Instant::now() < deadline, "shared creation never reached its first frame");
            if !ready { return; }
            {
                let seen = self.seen.lock().unwrap();
                assert!(seen.order.contains(&"draw"));
                let mut awaiting_geometry = false;
                for mark in &seen.order {
                    match *mark {
                        "window" => awaiting_geometry = true,
                        "geometry" => awaiting_geometry = false,
                        "draw" | "focus" => assert!(!awaiting_geometry, "geometry precedes draw/focus: {:?}", seen.order),
                        _ => {}
                    }
                }
            }
            let h = &mut self.host;
            h.win.as_mut().unwrap().surface = None;
            let _gpu = h.gpu.take();
            h.keep_running = false;
            let before = self.seen.lock().unwrap().order.len();
            self.seen.lock().unwrap().admission = Admission::Close;
            assert!(h.finish_creation(el), "a closing geometry callback stops the caller before its first draw");
            assert!(h.life.finished());
            let seen = self.seen.lock().unwrap();
            assert!(!seen.order[before..].iter().any(|mark| matches!(*mark, "draw" | "focus")));
            self.phase = Phase::Done;
            println!("SHARED CREATION PASS {:?}", seen.order);
            el.exit();
        }
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event().with_wayland().with_any_thread(true).build().unwrap();
    let seen = Arc::new(Mutex::new(Created::default()));
    let state = seen.clone();
    let window = seen.clone();
    let drawn = seen.clone();
    let ready = seen.clone();
    let created = seen.clone();
    let app = crate::App::new(AppSpec::new("Shared creation", (300.0, 200.0), || Arc::new(Mutex::new(Created::default())), |_, _| crate::el()))
        .init(move || state)
        .keep_running()
        .float_on_open()
        .on_window(move |_| window.lock().unwrap().order.push("window"))
        .on_window_geometry(|s, geometry, tx| {
            let mut seen = s.lock().unwrap();
            seen.geometry.push(geometry);
            seen.order.push("geometry");
            if matches!(seen.admission, Admission::Close) { tx.close_window(); }
            false
        })
        .on_focus(|s, focus, _| s.lock().unwrap().order.push(if focus { "focus" } else { "blur" }))
        .on_frame_drawn(move |_| drawn.lock().unwrap().order.push("draw"))
        .on_ready(move |_| ready.lock().unwrap().order.push("ready"));
    let spec = WindowSpec::from_app("shared-creation", app).on_created(move |_| created.lock().unwrap().order.push("created"));
    let host = test_host(spec, &event_loop, true);
    event_loop.run_app(&mut Probe { host, seen, phase: Phase::Waiting(Instant::now() + Duration::from_secs(10)) }).unwrap();
}
