use super::Host;
use crate::{AppSpec, WindowSpec, app::WindowRequest, render::Gpu, tooling::Args};
use crate::window::{AnyHost, Start, WakeEvent, gpu_failure};
use std::{cell::RefCell, rc::Rc, sync::Weak, time::Duration};
use winit::{application::ApplicationHandler, event::WindowEvent, event_loop::{ActiveEventLoop, EventLoopProxy}, window::WindowId};
use crate::platform::activation::{Activator, Attention, Counts, Wait};
use crate::platform::attention::{State, step, Event};
use winit::platform::wayland::EventLoopBuilderExtWayland;

const LIMIT: Duration = Duration::from_secs(5);

fn make(proxy: EventLoopProxy<WakeEvent>, pool: Rc<RefCell<Weak<Gpu>>>, key: &str, wait: bool) -> Host {
    let app = crate::App::new(AppSpec::new("Attention probe", (300.0, 200.0), || (), |_, _| crate::text("Ready")));
    let app = if wait { app.attention_wait(LIMIT) } else { app };
    let spec = WindowSpec::from_app(key, app);
    let incoming = crate::open_url::incoming::listen(&spec.app);
    Host::new(spec, None, Args::default(), Start {
        proxy, slot: 0, gpu: pool, wayland: true, traced: false,
        overlays: &mut Vec::new(), on_display_close: &mut Vec::new(),
    }, true, incoming)
}

fn attention(h: &mut Host) -> &mut Attention { &mut h.win.as_mut().expect("a shown window").native.attention }

fn counts(h: &mut Host) -> Option<Counts> {
    match attention(h) {
        State::Ready(a) | State::Unresponsive(a) => Some(a.counts()),
        _ => None,
    }
}

fn phase(h: &mut Host) -> &'static str {
    match attention(h) {
        State::Unprobed => "unprobed",
        State::Unavailable => "unavailable",
        State::Ready(_) => "ready",
        State::Unresponsive(_) => "unresponsive",
    }
}

fn live_tokens(h: &mut Host) -> usize {
    match attention(h) {
        State::Ready(a) | State::Unresponsive(a) => a.live_tokens(),
        _ => 0,
    }
}

/// Runs `f` and says how long it took.
fn timed(f: impl FnOnce()) -> Duration {
    let start = std::time::Instant::now();
    f();
    start.elapsed()
}

/// An exchange whose deadline has passed: the token request goes out, the wait times out.
fn force_timeout(h: &mut Host) {
    let State::Ready(a) = attention(h) else { panic!("force_timeout needs a ready window") };
    assert_eq!(a.attend(crate::time::now()), Wait::TimedOut);
    let event: Event<Activator> = Event::TimedOut;
    step(attention(h), event);
    assert_eq!(phase(h), "unresponsive");
}

fn checks(el: &ActiveEventLoop, h: &mut Host, proxy: &EventLoopProxy<WakeEvent>, pool: &Rc<RefCell<Weak<Gpu>>>) {
    let stale = || crate::time::now() - LIMIT;

    // n1: an idle window's attention completes, twice, without waiting for winit.
    assert_eq!(phase(h), "unprobed");
    let took = timed(|| h.focus(el, crate::time::now()));
    assert!(took < Duration::from_secs(1), "{took:?}");
    assert_eq!(phase(h), "ready");
    assert_eq!(counts(h), Some(Counts { exchanges: 1, activates: 1, syncs: 1 }));
    assert_eq!(live_tokens(h), 0, "the token is destroyed at done");
    h.focus(el, crate::time::now());
    assert_eq!((phase(h), counts(h).unwrap().exchanges), ("ready", 2), "the connection is still sound");
    println!("ATTENTION N1 PASS {took:?}");

    // n6: a request handled after its deadline doesn't run an exchange.
    let before = counts(h).unwrap();
    let took = timed(|| h.focus(el, stale()));
    assert!(took < Duration::from_millis(100), "{took:?}");
    assert_eq!((phase(h), counts(h).unwrap()), ("ready", before));
    h.window_request(el, WindowRequest::Open(None), stale());
    assert_eq!((phase(h), counts(h).unwrap()), ("ready", before), "an open's focus keeps the stamp");
    h.window_request(el, WindowRequest::Open(Some("stale-token".into())), stale());
    let after = counts(h).unwrap();
    assert_eq!(after, Counts { activates: before.activates + 1, ..before }, "a stale token is still handed over");
    assert_eq!(phase(h), "ready");
    println!("ATTENTION N6 PASS");

    // n3: a forwarded token waits for the compositor; after a timeout it is still sent.
    let before = counts(h).unwrap();
    h.window_request(el, WindowRequest::Open(Some("forwarded".into())), crate::time::now());
    assert_eq!(counts(h).unwrap(), Counts { activates: before.activates + 1, syncs: before.syncs + 1, ..before });
    force_timeout(h);
    let before = counts(h).unwrap();
    h.window_request(el, WindowRequest::Open(Some("after-timeout".into())), crate::time::now());
    assert_eq!(counts(h).unwrap(), Counts { activates: before.activates + 1, ..before });
    println!("ATTENTION N3 PASS");

    // n2: the timed-out window's in-flight token goes with it; a new window starts over.
    assert!(phase(h) == "unresponsive" || phase(h) == "ready");
    h.hide(el);
    assert!(h.win.is_none(), "a Wayland hide destroys the window");
    h.show(el, crate::time::now());
    assert_eq!(phase(h), "unprobed");
    h.focus(el, crate::time::now());
    assert_eq!(phase(h), "ready");
    println!("ATTENTION N2 PASS");

    // n6, stale forwarded token on a fresh window: no bind at all.
    h.hide(el);
    h.show(el, crate::time::now());
    h.window_request(el, WindowRequest::Open(Some("stale-unprobed".into())), stale());
    assert_eq!(phase(h), "unprobed", "a stale request never binds");
    println!("ATTENTION N6 UNPROBED PASS");

    // n4: without `attention_wait`, focus binds nothing and never waits.
    let mut plain = make(proxy.clone(), pool.clone(), "plain", false);
    plain.resume(el);
    let took = timed(|| plain.focus(el, crate::time::now()));
    assert!(took < Duration::from_millis(100), "{took:?}");
    assert_eq!(phase(&mut plain), "unprobed");
    println!("ATTENTION N4 PASS");

    // n5: after a terminal GPU failure, attention neither reopens the window nor clears it.
    plain.gpu_failure = gpu_failure::Policy::Failed;
    plain.hide(el);
    plain.window_request(el, WindowRequest::Open(None), crate::time::now());
    plain.focus(el, crate::time::now());
    assert!(plain.win.is_none() && matches!(plain.gpu_failure, gpu_failure::Policy::Failed));
    plain.release();
    println!("ATTENTION N5 PASS");
}

enum Phase {
    Starting,
    /// n1, last part: after the checks read the shared socket, winit still delivers the
    /// window's events. From the second redraw on, each waits for the compositor's frame
    /// callback, a Wayland event on winit's own queue.
    Redraws(u32),
    Done,
}

struct Probe {
    host: Option<Host>,
    proxy: EventLoopProxy<WakeEvent>,
    pool: Rc<RefCell<Weak<Gpu>>>,
    phase: Phase,
}

impl ApplicationHandler<WakeEvent> for Probe {
    fn resumed(&mut self, el: &ActiveEventLoop) { self.host.as_mut().unwrap().resume(el); }

    fn user_event(&mut self, el: &ActiveEventLoop, event: WakeEvent) { self.host.as_mut().unwrap().handle_user_event(el, event); }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let h = self.host.as_mut().unwrap();
        if !h.owns(id) { return; }
        let redraw = matches!(event, WindowEvent::RedrawRequested);
        h.handle_window_event(el, id, event);
        if let (Phase::Redraws(n), true) = (&self.phase, redraw) {
            if *n == 2 {
                self.phase = Phase::Done;
                println!("ATTENTION N1 EVENTS PASS");
                el.exit();
            } else {
                self.phase = Phase::Redraws(n + 1);
                if let Some(w) = &h.win { w.window.request_redraw(); }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let h = self.host.as_mut().unwrap();
        h.about_to_wait(el);
        if matches!(self.phase, Phase::Starting) && h.native_handle.is_ready() {
            checks(el, h, &self.proxy, &self.pool);
            h.win.as_ref().unwrap().window.request_redraw();
            self.phase = Phase::Redraws(0);
            println!("ATTENTION WAITING FOR REDRAWS, flow {:?}", el.control_flow());
        }
    }
}

#[test]
#[ignore = "requires an isolated Wayland compositor"]
#[allow(clippy::disallowed_methods, reason = "native test checks its isolation guard")]
fn native_attention_completes_before_returning() {
    assert_eq!(std::env::var("RUNE_ATTENTION_NATIVE_TEST").as_deref(), Ok("1"));
    assert!(std::env::var_os("DISPLAY").is_none());
    // A hung step can't hold the run: the loop waits on the compositor alone. Ending the
    // test (pass or panic) drops `_done`, which stops the watchdog.
    let (_done, watched) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        if watched.recv_timeout(Duration::from_secs(60)) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
            eprintln!("attention native test timed out");
            std::process::abort();
        }
    });
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event().with_wayland().with_any_thread(true).build().unwrap();
    let proxy = event_loop.create_proxy();
    let pool = Rc::new(RefCell::new(Weak::new()));
    let host = make(proxy.clone(), pool.clone(), "attention", true);
    let mut probe = Probe { host: Some(host), proxy, pool, phase: Phase::Starting };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| event_loop.run_app(&mut probe)));
    // Surfaces must die before winit's display does.
    probe.host.as_mut().unwrap().release();
    match result {
        Ok(result) => {
            result.unwrap();
            assert!(matches!(probe.phase, Phase::Done));
            println!("ATTENTION EXIT PASS");
        }
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
