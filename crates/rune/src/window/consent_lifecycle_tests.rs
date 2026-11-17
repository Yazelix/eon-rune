use super::*;
use crate::quit::request::{self, NativeReply};
use std::io::Write;
use std::sync::OnceLock;
use winit::platform::wayland::EventLoopBuilderExtWayland;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case { Allow, TxClose, TxQuit, Default, Multi, Quit, QuitInHook }

#[derive(Default)]
struct WindowSeen {
    ready: bool,
    requests: usize,
    frames: usize,
    presented_request: usize,
    quit_requests: usize,
    events: Vec<&'static str>,
}

struct Seen {
    case: Case,
    windows: [WindowSeen; 2],
}

struct State {
    index: usize,
    seen: Arc<Mutex<Seen>>,
}

static FINAL: OnceLock<Arc<Mutex<Seen>>> = OnceLock::new();

fn verify_final() {
    let seen = FINAL.get().unwrap().lock().unwrap();
    let first = &seen.windows[0];
    let second = &seen.windows[1];
    let events: &[&str] = match seen.case {
        Case::TxQuit => &["quit"],
        Case::Quit | Case::QuitInHook => &["quit", "close"],
        _ => &["close"],
    };
    assert_eq!(first.events, events);
    assert_eq!(first.requests, if matches!(seen.case, Case::Default | Case::Quit | Case::QuitInHook) { 0 } else { 3 });
    match seen.case {
        Case::Multi => assert_eq!(second.events, ["quit"]),
        Case::Quit | Case::QuitInHook => assert_eq!(second.events, ["quit", "close"]),
        _ => assert!(second.events.is_empty()),
    }
    let expected_requests = match seen.case { Case::Quit => [2, 2], Case::QuitInHook => [1, 1], _ => [0, 0] };
    assert_eq!([first.quit_requests, second.quit_requests], expected_requests);
    let mut out = std::io::stdout().lock();
    writeln!(out, "CONSENT FINAL PASS {:?}", seen.case).unwrap();
    out.flush().unwrap();
}

fn make(index: usize, seen: Arc<Mutex<Seen>>) -> (AnyWindowSpec, crate::AppHandle<State>) {
    let case = seen.lock().unwrap().case;
    let initial = seen.clone();
    let title = if index == 0 { "Consent lifecycle main" } else { "Consent lifecycle second" };
    let app = crate::App::new(AppSpec::new(title, (300.0, 200.0), || panic!("explicit init"), |s: &State, _| {
        let requests = s.seen.lock().unwrap().windows[s.index].requests;
        crate::text(if requests == 0 { "Ready" } else { "Awaiting consent" })
    }))
        .init(move || State { index, seen: initial })
        .float_on_open()
        .on_ready(move |_| seen.lock().unwrap().windows[index].ready = true)
        .on_frame_drawn(|rt| {
            let mut seen = rt.state.seen.lock().unwrap();
            let row = &mut seen.windows[rt.state.index];
            row.frames += 1;
            if row.frames == 1 || row.presented_request != row.requests {
                println!("CONSENT FRAME index={} frame={} request={}", rt.state.index, row.frames, row.requests);
            }
            row.presented_request = row.requests;
        })
        .on_close(|s, _| s.seen.lock().unwrap().windows[s.index].events.push("close"))
        .on_quit(|s, _| s.seen.lock().unwrap().windows[s.index].events.push("quit"))
        .on_quit_request(|s, tx| {
            let mut seen = s.seen.lock().unwrap();
            let case = seen.case;
            let row = &mut seen.windows[s.index];
            row.quit_requests += 1;
            if s.index == 1 && row.quit_requests == 1 {
                if case == Case::QuitInHook { tx.quit(); }
                crate::QuitResponse::Defer
            } else {
                crate::QuitResponse::Quit
            }
        });
    let app = if case == Case::Default { app } else {
        app.on_close_request(|s, tx| {
            let mut seen = s.seen.lock().unwrap();
            let case = seen.case;
            let row = &mut seen.windows[s.index];
            row.requests += 1;
            println!("CONSENT REQUEST index={} request={}", s.index, row.requests);
            tx.instant();
            if row.requests < 3 { return crate::CloseResponse::Defer; }
            match case {
                Case::TxClose => tx.close_window(),
                Case::TxQuit => tx.quit(),
                _ => return crate::CloseResponse::Close,
            }
            crate::CloseResponse::Defer
        })
    };
    let handle = app.handle();
    (WindowSpec::from_app(if index == 0 { "main" } else { "second" }, app).into(), handle)
}

enum Phase { Admission, FirstClose, SecondClose, AllowedClose, HideRemaining, Hidden, Veto, Permit, Done }

struct Probe {
    windows: Windows,
    seen: Arc<Mutex<Seen>>,
    handles: Vec<crate::AppHandle<State>>,
    phase: Phase,
    deadline: Instant,
}

impl Probe {
    fn begin_quit(&mut self) {
        assert_eq!(request::request(|| self.windows.proxy.send_event(WakeEvent::Quit(false)).is_ok()), NativeReply::Later);
        assert_eq!(request::request(|| panic!("duplicate must not queue")), NativeReply::Cancel);
        assert!(!crate::quit::quitting(), "consent cannot start teardown or its watchdog");
    }
}

impl ApplicationHandler<WakeEvent> for Probe {
    fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) { self.windows.new_events(el, cause); }
    fn resumed(&mut self, el: &ActiveEventLoop) { self.windows.resumed(el); }
    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.windows.window_event(el, id, event);
    }
    fn user_event(&mut self, el: &ActiveEventLoop, event: WakeEvent) {
        let quit = matches!(event, WakeEvent::Quit(false));
        self.windows.user_event(el, event);
        if !quit { return; }
        assert!(!el.exiting());
        let allow = request::flush().expect("one pending native reply");
        assert_eq!(request::flush(), None, "a native reply is delivered once");
        assert_eq!(crate::quit::quitting(), self.seen.lock().unwrap().case == Case::QuitInHook, "only explicit Quit can start teardown during consent");
        assert!(self.seen.lock().unwrap().windows.iter().all(|row| row.events.is_empty()));
        assert!(self.windows.hosts.iter().flatten().all(|h| !h.finished()));
        assert!(!self.windows.hosts[1].as_ref().unwrap().reopen_candidate().visible, "hidden host stayed hidden during consent");
        if allow {
            assert!(matches!(self.phase, Phase::Permit));
            // Linux exercises the delivery/teardown seam; AppKit owns this call on macOS.
            self.windows.exiting(el);
            self.phase = Phase::Done;
            el.exit();
        } else {
            assert!(matches!(self.phase, Phase::Veto));
            println!("QUIT VETO PASS");
            self.begin_quit();
            self.phase = Phase::Permit;
        }
    }
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.windows.about_to_wait(el);
        assert!(Instant::now() < self.deadline, "consent lifecycle timed out");
        let case = self.seen.lock().unwrap().case;
        match self.phase {
            Phase::Admission if self.seen.lock().unwrap().windows[..self.handles.len()].iter().all(|row| row.ready) => {
                if matches!(case, Case::Quit | Case::QuitInHook) {
                    self.handles[1].hide();
                    self.phase = Phase::Hidden;
                } else {
                    self.phase = Phase::FirstClose;
                    println!("CONSENT CLOSE READY");
                }
            }
            Phase::FirstClose | Phase::SecondClose => {
                let seen = self.seen.lock().unwrap();
                let row = &seen.windows[0];
                let expected = if matches!(self.phase, Phase::FirstClose) { 1 } else { 2 };
                if row.requests == expected && row.presented_request == expected {
                    assert!(row.events.is_empty());
                    assert!(self.windows.hosts[0].as_ref().unwrap().reopen_candidate().visible);
                    assert!(!crate::quit::quitting());
                    assert_eq!(row.quit_requests, 0);
                    println!("CONSENT CANCELLED {expected}");
                    self.phase = if expected == 1 { Phase::SecondClose } else { Phase::AllowedClose };
                }
            }
            Phase::AllowedClose if case == Case::Multi && self.windows.hosts[0].is_none() => {
                assert!(self.windows.hosts[1].as_ref().unwrap().reopen_candidate().visible);
                assert!(!crate::quit::quitting());
                self.handles[1].hide();
                self.phase = Phase::HideRemaining;
            }
            Phase::HideRemaining if !self.windows.hosts[1].as_ref().unwrap().reopen_candidate().visible => {
                assert!(self.seen.lock().unwrap().windows[1].events.is_empty());
                self.handles[1].quit();
                self.phase = Phase::Done;
            }
            Phase::Hidden if !self.windows.hosts[1].as_ref().unwrap().reopen_candidate().visible => {
                self.begin_quit();
                self.phase = if case == Case::QuitInHook { Phase::Permit } else { Phase::Veto };
            }
            _ => {}
        }
        let flow = match el.control_flow() {
            ControlFlow::Wait => ControlFlow::WaitUntil(self.deadline),
            ControlFlow::WaitUntil(at) => ControlFlow::WaitUntil(at.min(self.deadline)),
            flow => flow,
        };
        el.set_control_flow(flow);
    }
    fn exiting(&mut self, el: &ActiveEventLoop) {
        if !matches!(self.seen.lock().unwrap().case, Case::Quit | Case::QuitInHook) { self.windows.exiting(el); }
    }
}

#[test]
#[ignore = "requires isolated consent lifecycle driver; one child per exit mode/case"]
fn native_consent_exit_lifecycle() {
    exit_lifecycle(crate::orderly_exit, true);
}

#[allow(clippy::disallowed_methods, reason = "native test checks its isolation guard and child mode")]
fn exit_lifecycle(orderly: fn(), default_fast: bool) {
    assert_eq!(std::env::var("RUNE_CONSENT_NATIVE_TEST").as_deref(), Ok("1"));
    let mode = std::env::var("RUNE_CONSENT_EXIT").unwrap();
    assert!(matches!(mode.as_str(), "fast" | "orderly"));
    assert_eq!(crate::quit::fast_exit_hook().is_some(), default_fast);
    if mode == "orderly" { orderly(); }
    let case = match std::env::var("RUNE_CONSENT_CASE").unwrap().as_str() {
        "allow" => Case::Allow, "tx-close" => Case::TxClose, "tx-quit" => Case::TxQuit,
        "default" => Case::Default, "multi" => Case::Multi, "quit" => Case::Quit,
        "quit-in-hook" => Case::QuitInHook, _ => panic!("unknown consent case"),
    };
    let seen = Arc::new(Mutex::new(Seen { case, windows: Default::default() }));
    assert!(FINAL.set(seen.clone()).is_ok());
    if mode == "fast" { crate::fast_exit(verify_final); }
    assert_eq!(crate::quit::fast_exit_hook().is_some(), mode == "fast");
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event().with_wayland().with_any_thread(true).build().unwrap();
    let mut specs = Vec::new();
    let mut handles = Vec::new();
    for index in 0..if matches!(case, Case::Multi | Case::Quit | Case::QuitInHook) { 2 } else { 1 } {
        let (spec, handle) = make(index, seen.clone());
        specs.push(spec);
        handles.push(handle);
    }
    let windows = Windows { factory: Some(Box::new(move |_| specs)), args: Args::default(), proxy: event_loop.create_proxy(), hosts: Vec::new(), gpu: Rc::new(RefCell::new(Weak::new())) };
    event_loop.run_app(&mut Probe { windows, seen, handles, phase: Phase::Admission, deadline: Instant::now() + Duration::from_secs(20) }).unwrap();
    assert_eq!(mode, "orderly", "fast mode must finish through the real process exit");
    verify_final();
    println!("CONSENT RETURN PASS");
}
