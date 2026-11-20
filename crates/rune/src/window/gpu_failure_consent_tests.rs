use super::*;
use crate::quit::request::{self, NativeReply};
use std::sync::atomic::AtomicUsize;
use winit::platform::wayland::EventLoopBuilderExtWayland;

#[derive(Default)]
struct Seen {
    raw: usize,
    close_requests: usize,
    quit_requests: usize,
    frames: usize,
    notifications: Vec<&'static str>,
}

type State = Arc<std::sync::Mutex<Seen>>;

fn snapshot(state: &State) -> (usize, usize) {
    let seen = state.lock().unwrap();
    (seen.raw, seen.frames)
}

fn host(
    proxy: EventLoopProxy<WakeEvent>,
    terminal: bool,
) -> (Host<State>, crate::AppHandle<State>, State) {
    let seen = State::default();
    let initial = seen.clone();
    let app = crate::App::new(AppSpec::new(
        "Post-loss consent",
        (120.0, 80.0),
        || panic!("explicit init"),
        |_: &State, _| crate::el(),
    ))
    .init(move || initial)
    .args(Vec::new())
    .on_window_event(|s, _, _, _| {
        s.lock().unwrap().raw += 1;
        false
    })
    .on_frame_drawn(|rt| rt.state.lock().unwrap().frames += 1)
    .on_close_request(|s, _| {
        let mut seen = s.lock().unwrap();
        seen.close_requests += 1;
        if seen.close_requests == 1 {
            crate::CloseResponse::Defer
        } else {
            crate::CloseResponse::Close
        }
    })
    .on_quit_request(|s, _| {
        let mut seen = s.lock().unwrap();
        seen.quit_requests += 1;
        if seen.quit_requests == 1 {
            crate::QuitResponse::Defer
        } else {
            crate::QuitResponse::Quit
        }
    })
    .on_close(|s, _| s.lock().unwrap().notifications.push("close"))
    .on_quit(|s, _| s.lock().unwrap().notifications.push("quit"));
    let app = if terminal {
        app.terminal_gpu_loss()
    } else {
        app
    };
    let handle = app.handle();
    let host = Host::new(
        WindowSpec::from_app("post-loss", app),
        WindowHandle::new("post-loss"),
        Args::default(),
        proxy,
        0,
        Rc::default(),
        true,
    );
    (host, handle, seen)
}

fn lose(host: &mut Host<State>, el: &ActiveEventLoop, reason: &str) -> (WindowId, Arc<Gpu>) {
    let id = host.win.as_ref().unwrap().window.id();
    let gpu = host.gpu.as_ref().unwrap().0.clone();
    eprintln!("GPU_LOSS_ADAPTER {:?}", gpu.adapter.get_info());
    gpu.failure.record(reason.into());
    host.handle_window_event(el, id, WindowEvent::CloseRequested);
    assert!(host.win.is_none() && host.gpu.is_none());
    assert!(matches!(host.hooks.gpu_failure, Policy::Failed));
    assert!(matches!(host.life, Life::Hidden { .. }));
    (id, gpu)
}

#[test]
#[ignore = "requires a private headless Wayland session"]
#[allow(
    clippy::disallowed_methods,
    reason = "explicit private-session native test gate"
)]
fn close_and_quit_after_terminal_gpu_loss() {
    assert_eq!(std::env::var("RUNE_GPU_FAILURE_NATIVE").as_deref(), Ok("1"));
    crate::orderly_exit();
    static STARTS: AtomicUsize = AtomicUsize::new(0);
    crate::on_gpu(|event| {
        if event == crate::GpuEvent::Starting {
            STARTS.fetch_add(1, Ordering::SeqCst);
        }
    });
    struct Probe {
        proxy: EventLoopProxy<WakeEvent>,
        done: bool,
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            // A healthy terminal app and a Recover app with an uncaptured-error record retain consent.
            for terminal in [true, false] {
                let (mut control, _, seen) = host(self.proxy.clone(), terminal);
                control.resume(el);
                let id = control.win.as_ref().unwrap().window.id();
                if !terminal {
                    control
                        .gpu
                        .as_ref()
                        .unwrap()
                        .0
                        .failure
                        .record("recover control".into());
                }
                assert!(!control.gpu_failed());
                control.handle_window_event(el, id, WindowEvent::CloseRequested);
                assert!(control.life.shown());
                assert_eq!(seen.lock().unwrap().close_requests, 1);
                assert!(seen.lock().unwrap().notifications.is_empty());
                control.handle_window_event(el, id, WindowEvent::CloseRequested);
                assert!(control.life.finished());
                assert_eq!(seen.lock().unwrap().close_requests, 2);
                assert_eq!(seen.lock().unwrap().notifications, ["close"]);
            }

            let (mut closing, close_handle, closed) = host(self.proxy.clone(), true);
            closing.resume(el);
            let id = closing.win.as_ref().unwrap().window.id();
            closing.handle_window_event(el, id, WindowEvent::CloseRequested);
            assert_eq!(
                closed.lock().unwrap().close_requests,
                1,
                "consent deferred before loss"
            );
            let before = snapshot(&closed);
            let starts = STARTS.load(Ordering::SeqCst);
            let (old_id, _gpu) = lose(&mut closing, el, "terminal close failure");
            assert_eq!(
                closed.lock().unwrap().close_requests,
                1,
                "loss preempts native consent"
            );
            closing.handle_window_event(
                el,
                old_id,
                WindowEvent::Ime(winit::event::Ime::Commit("blocked".into())),
            );
            closing.handle_window_event(el, old_id, WindowEvent::CloseRequested);
            closing.open(el, None, Instant::now());
            closing.redraw();
            assert_eq!(snapshot(&closed), before);
            assert!(!close_handle.is_visible());
            assert_eq!(
                close_handle.window_error().as_deref(),
                Some("terminal close failure")
            );
            assert_eq!(
                STARTS.load(Ordering::SeqCst),
                starts,
                "no replacement after loss"
            );
            close_handle.close();
            closing.drain(el, false);
            close_handle.close();
            closing.drain(el, false);
            assert!(closing.life.finished());
            assert_eq!(
                closed.lock().unwrap().close_requests,
                1,
                "programmatic permission bypasses consent"
            );
            assert_eq!(closed.lock().unwrap().notifications, ["close"]);
            drop(closing);

            let (mut quitting, quit_handle, seen) = host(self.proxy.clone(), true);
            quitting.resume(el);
            let before = snapshot(&seen);
            let starts = STARTS.load(Ordering::SeqCst);
            let (old_id, _gpu) = lose(&mut quitting, el, "terminal quit failure");
            quitting.handle_window_event(
                el,
                old_id,
                WindowEvent::Ime(winit::event::Ime::Commit("blocked".into())),
            );
            quitting.open(el, None, Instant::now());
            quitting.redraw();
            assert_eq!(snapshot(&seen), before);
            for allow in [false, true] {
                assert_eq!(request::request(|| true), NativeReply::Later);
                assert_eq!(
                    request::request(|| panic!("duplicate Quit must not queue")),
                    NativeReply::Cancel
                );
                assert!(request::dispatch([&mut quitting], |h| h.quit_requested(el)));
                assert_eq!(request::flush(), Some(allow));
                assert_eq!(request::flush(), None, "one native reply per attempt");
                assert!(!el.exiting() && !crate::quit::quitting());
                assert!(seen.lock().unwrap().notifications.is_empty());
            }
            assert_eq!(seen.lock().unwrap().quit_requests, 2);
            assert_eq!(seen.lock().unwrap().close_requests, 0);
            assert_eq!(STARTS.load(Ordering::SeqCst), starts);
            assert_eq!(
                quit_handle.window_error().as_deref(),
                Some("terminal quit failure")
            );
            let mut windows = Windows {
                factory: None,
                args: Args::default(),
                proxy: self.proxy.clone(),
                hosts: vec![Some(Box::new(quitting))],
                gpu: Rc::default(),
            };
            // The shared AppKit-allow lifecycle seam, before winit itself reports exiting.
            windows.exiting(el);
            windows.exiting(el);
            assert_eq!(seen.lock().unwrap().notifications, ["quit", "close"]);
            assert_eq!(seen.lock().unwrap().quit_requests, 2);
            assert_eq!(seen.lock().unwrap().close_requests, 0);
            assert_eq!(STARTS.load(Ordering::SeqCst), starts);
            assert!(!request::registered());
            self.done = true;
            println!(
                "POST-LOSS CONSENT PASS: native guard, explicit close, Quit defer/allow, once-only notifications, Recover/no-loss controls"
            );
            el.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event()
        .with_wayland()
        .with_any_thread(true)
        .build()
        .unwrap();
    let mut probe = Probe {
        proxy: event_loop.create_proxy(),
        done: false,
    };
    event_loop.run_app(&mut probe).unwrap();
    assert!(probe.done, "post-run assertion must execute");
}
