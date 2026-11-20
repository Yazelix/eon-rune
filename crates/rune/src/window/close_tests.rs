use super::*;
use crate::CloseResponse;

#[test]
fn close_consent_is_opt_in_and_survives_window_spec() {
    let app = crate::App::new(AppSpec::new(
        "close",
        (20.0, 20.0),
        || (),
        |_, _| crate::el(),
    ));
    assert!(app.hooks.close_request.is_none());
    let spec = WindowSpec::from_app("second", app.on_close_request(|_, _| CloseResponse::Defer));
    assert!(spec.app.hooks.close_request.is_some());
}

#[test]
fn attention_wait_is_opt_in_and_reaches_host() {
    let (app, limit) = (|| crate::App::new(AppSpec::new("attention", (20.0, 20.0), || (), |_, _| crate::el())), Duration::from_secs(5));
    assert_eq!(app().attention_wait, None);
    let spec = WindowSpec::from_app("attention", app().attention_wait(limit));
    assert_eq!(spec.app.attention_wait, Some(limit));
}

#[cfg(target_os = "linux")]
mod native {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use winit::platform::wayland::EventLoopBuilderExtWayland;

    #[derive(Default)]
    enum Answer {
        #[default]
        Defer,
        Quiet,
        Close,
        CloseInTx,
        QuitInTx,
    }

    #[derive(Default)]
    struct Seen {
        answer: Answer,
        calls: usize,
        closes: usize,
        order: Vec<&'static str>,
        prompt_presents: Arc<AtomicUsize>,
    }

    fn consent(s: &mut Seen, tx: &mut Tx) -> CloseResponse {
        s.calls += 1;
        s.order.push("consent");
        tx.instant();
        match s.answer {
            Answer::Close => return CloseResponse::Close,
            Answer::CloseInTx => tx.close_window(),
            Answer::QuitInTx => tx.quit(),
            Answer::Quiet => tx.live_only(),
            Answer::Defer => tx.wake_after(Duration::from_millis(20)),
        }
        CloseResponse::Defer
    }

    fn make(
        proxy: EventLoopProxy<WakeEvent>,
        pool: Rc<RefCell<Weak<Gpu>>>,
        key: &str,
    ) -> (Host<Seen>, crate::AppHandle<Seen>) {
        let app = crate::App::new(AppSpec::new(
            "Close consent probe",
            (300.0, 200.0),
            Seen::default,
            |s, _| {
                crate::text(if s.calls == 0 {
                    "Ready"
                } else {
                    "Close? Waiting for approval"
                })
                .key("close-prompt")
            },
        ))
        .on_close_request(consent)
        .on_close(|s, _| {
            s.closes += 1;
            s.order.push("teardown");
        })
        .on_frame_drawn(|rt| {
            assert!(rt.find("close-prompt").is_some());
            rt.state
                .prompt_presents
                .store(rt.state.calls, Ordering::SeqCst);
        });
        let handle = app.handle();
        (
            Host::new(
                WindowSpec::from_app(key, app),
                WindowHandle::new(key),
                Args::default(),
                proxy,
                0,
                pool,
                true,
            ),
            handle,
        )
    }

    // These are injected dispatch controls, not compositor-close witnesses.
    fn close_controls(
        el: &ActiveEventLoop,
        proxy: &EventLoopProxy<WakeEvent>,
        pool: &Rc<RefCell<Weak<Gpu>>>,
    ) {
        for path in [
            "window-request",
            "app-handle",
            "shell",
            "teardown",
            "default",
            "kept-hidden",
        ] {
            let (mut h, handle) = make(proxy.clone(), pool.clone(), path);
            h.started = true;
            h.life.show();
            match path {
                "window-request" => h.window_request(el, WindowRequest::Close, Instant::now()),
                "app-handle" => {
                    handle.close();
                    h.drain(el, false);
                }
                "shell" => {
                    h.rt.update(|_, tx| tx.close_window());
                    h.shell(el);
                }
                "teardown" => {
                    h.terminate();
                    h.terminate();
                }
                "default" => {
                    h.hooks.close_request = None;
                    assert!(!h.close_requested(el));
                    h.close(el);
                }
                "kept-hidden" => {
                    h.keep_running = true;
                    h.close(el);
                    assert!(!h.life.shown() && !h.life.finished());
                    assert!(h.close_requested(el));
                    h.life.show();
                    h.rt.state.answer = Answer::Close;
                    assert!(h.close_requested(el));
                    assert_eq!(h.rt.state.closes, 2);
                    assert_eq!(h.rt.state.calls, 1);
                    continue;
                }
                _ => unreachable!(),
            }
            assert_eq!(h.rt.state.calls, 0, "{path} must bypass consent");
            assert_eq!(h.rt.state.closes, 1, "{path} tears down once");
            assert!(h.life.finished(), "{path}");
        }
        for answer in [Answer::Close, Answer::CloseInTx] {
            let (mut h, _) = make(proxy.clone(), pool.clone(), "callback-close");
            h.life.show();
            h.rt.state.answer = answer;
            assert!(h.close_requested(el));
            assert!(h.life.finished());
            assert_eq!(h.rt.state.order, ["consent", "teardown"]);
            assert!(h.close_requested(el));
            assert_eq!((h.rt.state.calls, h.rt.state.closes), (1, 1));
        }
        println!("CLOSE INJECTED BYPASS PASS");
    }

    // Exit requests run last: the active loop's exit flag cannot be reset. No relaunch is spawned.
    fn exit_controls(
        el: &ActiveEventLoop,
        proxy: &EventLoopProxy<WakeEvent>,
        pool: &Rc<RefCell<Weak<Gpu>>>,
    ) {
        assert!(
            !el.exiting(),
            "callback-initiated quit starts from a running loop"
        );
        for path in [
            "callback-quit",
            "shell-quit",
            "shell-relaunch",
            "handle-quit",
            "handle-relaunch",
        ] {
            let (mut h, handle) = make(proxy.clone(), pool.clone(), path);
            h.started = true;
            h.life.show();
            crate::quit::send(crate::quit::Event::Cancel);
            match path {
                "shell-quit" => {
                    h.rt.update(|_, tx| tx.quit());
                    h.shell(el);
                }
                "shell-relaunch" => {
                    h.rt.update(|_, tx| tx.relaunch());
                    h.shell(el);
                }
                "handle-quit" => {
                    handle.quit();
                    h.drain(el, false);
                }
                "handle-relaunch" => {
                    handle.relaunch();
                    h.drain(el, false);
                }
                "callback-quit" => {
                    h.rt.state.answer = Answer::QuitInTx;
                    assert!(h.close_requested(el));
                }
                _ => unreachable!(),
            }
            assert!(el.exiting(), "{path}");
            let after = if path.ends_with("relaunch") {
                crate::quit::After::Relaunch
            } else {
                crate::quit::After::Exit
            };
            assert_eq!(
                crate::quit::state(),
                crate::quit::State::Quitting(crate::quit::Hooks::Owed, after)
            );
            assert_eq!(
                h.rt.state.calls,
                usize::from(path == "callback-quit"),
                "{path} must not re-enter consent"
            );
            assert_eq!(
                h.rt.state.closes, 0,
                "{path} preserves exit notification behavior"
            );
            let calls = h.rt.state.calls;
            assert!(h.close_requested(el));
            assert_eq!(
                h.rt.state.calls, calls,
                "consent must not run after exit was requested: {path}"
            );
        }
        crate::quit::send(crate::quit::Event::Cancel);
        println!("CLOSE INJECTED EXIT PASS");
    }

    fn deferred_last_window_then_another_main(
        el: &ActiveEventLoop,
        proxy: &EventLoopProxy<WakeEvent>,
        pool: &Rc<RefCell<Weak<Gpu>>>,
    ) {
        for permission in ["native", "handle", "tx"] {
            let (mut first, handle) = make(proxy.clone(), pool.clone(), "first-main");
            first.resume(el);
            first.rt.state.answer = Answer::Quiet;
            let id = first.win.as_ref().unwrap().window.id();
            first.closing_ends_app(true);
            first.handle_window_event(el, id, WindowEvent::CloseRequested);
            assert_eq!((first.rt.state.calls, first.rt.state.closes), (1, 0));
            assert!(first.life.shown());
            assert!(!first.ends_app, "deferred close must clear the last-window hint");

            let (mut second, _) = make(proxy.clone(), pool.clone(), "second-main");
            second.resume(el);
            assert!(first.keeps_loop() && second.keeps_loop());
            assert!(!second.finished() && second.life.shown());
            match permission {
                "native" => {
                    // A later native request takes the now-current two-window status.
                    first.closing_ends_app(false);
                    assert!(!first.ends_app);
                    first.rt.state.answer = Answer::Close;
                    first.handle_window_event(el, id, WindowEvent::CloseRequested);
                }
                "handle" => { handle.close(); first.drain(el, false); }
                "tx" => { first.rt.update(|_, tx| tx.close_window()); first.shell(el); }
                _ => unreachable!(),
            }
            assert!(first.finished() && !first.ends_app);
            assert_eq!(first.rt.state.closes, 1);
            assert_eq!(first.rt.state.calls, if permission == "native" { 2 } else { 1 });
            assert!(!el.exiting() && !second.finished() && second.life.shown());
            assert_eq!((second.rt.state.calls, second.rt.state.closes), (0, 0));
            first.release();
            second.release();
        }
        println!("CLOSE DEFER OPEN MAIN PASS");
    }

    enum Phase {
        Starting,
        FirstRequest,
        Prompt { call: usize },
        SecondRequest,
        Resolving,
        Done,
    }

    struct Probe {
        host: Option<Host<Seen>>,
        proxy: EventLoopProxy<WakeEvent>,
        pool: Rc<RefCell<Weak<Gpu>>>,
        phase: Phase,
        deadline: std::time::Instant,
    }

    impl Probe {
        fn finish(&mut self, el: &ActiveEventLoop) {
            let h = self.host.as_mut().unwrap();
            assert!(h.life.finished());
            assert_eq!(h.rt.state.order, ["consent", "consent", "teardown"]);
            assert_eq!((h.rt.state.calls, h.rt.state.closes), (2, 1));
            h.close(el);
            assert_eq!(
                h.rt.state.closes, 1,
                "duplicate programmatic close stays closed"
            );
            close_controls(el, &self.proxy, &self.pool);
            deferred_last_window_then_another_main(el, &self.proxy, &self.pool);
            // Acceptance on an actual native window skips a post-close redraw request.
            let (mut accepting, _) = make(self.proxy.clone(), self.pool.clone(), "accepting");
            accepting.resume(el);
            accepting.rt.state.answer = Answer::Close;
            let id = accepting.win.as_ref().unwrap().window.id();
            accepting.win.as_ref().unwrap().redraw.set(Redraw::Idle);
            accepting.handle_window_event(el, id, WindowEvent::CloseRequested);
            assert!(accepting.life.finished());
            assert_eq!(accepting.rt.state.order, ["consent", "teardown"]);
            assert!(matches!(
                accepting.win.as_ref().unwrap().redraw.get(),
                Redraw::Idle
            ));
            accepting.release();
            let (mut default, _) = make(self.proxy.clone(), self.pool.clone(), "default-native");
            default.hooks.close_request = None;
            default.resume(el);
            let id = default.win.as_ref().unwrap().window.id();
            default.handle_window_event(el, id, WindowEvent::CloseRequested);
            assert!(default.life.finished());
            assert_eq!((default.rt.state.calls, default.rt.state.closes), (0, 1));
            default.release();
            h.release();
            self.phase = Phase::Done;
            exit_controls(el, &self.proxy, &self.pool);
            println!("CLOSE CONSENT PASS");
            el.exit();
        }
    }

    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let h = self.host.as_mut().unwrap();
            h.resume(el);
            assert!(h.hooks.close_request.is_some(), "registration reaches Host");
            let w = h.win.as_mut().unwrap();
            let original = w.first_frame;
            // Inject an unfinished first-frame reveal that Hide would cancel.
            w.first_frame = FirstFrame::default();
            w.first_frame.request_reveal(true, false, Instant::now());
            let before = w.first_frame;
            let id = w.window.id();
            let life = h.life;
            let ready = h.native_handle.is_ready();
            h.rt.state.answer = Answer::Quiet;
            h.win.as_ref().unwrap().redraw.set(Redraw::Idle);
            h.handle_window_event(el, id, WindowEvent::CloseRequested);
            assert_eq!(
                h.win.as_ref().unwrap().first_frame,
                before,
                "deferred close preserves unfinished FirstFrame"
            );
            assert_eq!(
                (h.rt.state.calls, h.rt.state.closes),
                (1, 0),
                "deferred native close must consult consent before teardown"
            );
            assert_eq!(h.life, life);
            assert_eq!(h.native_handle.is_ready(), ready);
            assert!(
                matches!(h.win.as_ref().unwrap().redraw.get(), Redraw::Idle),
                "live_only does not request a prompt redraw"
            );
            h.win.as_mut().unwrap().first_frame = original;
            h.rt.state.calls = 0;
            h.rt.state.order.clear();
            h.rt.state.answer = Answer::Defer;
            h.win.as_ref().unwrap().request_redraw();
            el.set_control_flow(ControlFlow::WaitUntil(self.deadline));
            println!("CLOSE PREFRAME INJECTED PASS");
        }

        fn user_event(&mut self, el: &ActiveEventLoop, event: WakeEvent) {
            let h = self.host.as_mut().unwrap();
            h.handle_user_event(el, event);
            if matches!(self.phase, Phase::Resolving) && h.life.finished() {
                self.finish(el);
            }
        }

        fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            let h = self.host.as_mut().unwrap();
            if !h.owns(id) {
                return;
            }
            let close = matches!(event, WindowEvent::CloseRequested);
            let snapshot = h.win.as_ref().map(|w| {
                (
                    w.first_frame,
                    w.presented.is_some(),
                    w.window.id(),
                    h.native_handle.is_ready(),
                )
            });
            h.handle_window_event(el, id, event);
            if close {
                let w = h
                    .win
                    .as_ref()
                    .expect("deferred native close keeps its window");
                assert_eq!(
                    Some((
                        w.first_frame,
                        w.presented.is_some(),
                        w.window.id(),
                        h.native_handle.is_ready()
                    )),
                    snapshot
                );
                assert!(h.life.shown());
                assert_eq!(h.rt.state.closes, 0);
                assert!(
                    h.rt.wake_deadline().is_some(),
                    "callback timer is scheduled without another frame"
                );
                assert!(matches!(
                    self.phase,
                    Phase::FirstRequest | Phase::SecondRequest
                ));
                self.phase = Phase::Prompt {
                    call: h.rt.state.calls,
                };
                println!("CLOSE NATIVE DEFERRED {}", h.rt.state.calls);
            }
        }

        fn about_to_wait(&mut self, el: &ActiveEventLoop) {
            if matches!(self.phase, Phase::Done) {
                return;
            }
            assert!(
                std::time::Instant::now() < self.deadline,
                "isolated close probe timed out"
            );
            let h = self.host.as_mut().unwrap();
            h.about_to_wait(el);
            match self.phase {
                Phase::Starting if h.native_handle.is_ready() => {
                    self.phase = Phase::FirstRequest;
                    println!("CLOSE READY");
                }
                Phase::Prompt { call }
                    if h.rt.state.prompt_presents.load(Ordering::SeqCst) >= call =>
                {
                    assert!(h.life.shown());
                    assert_eq!(h.rt.state.closes, 0);
                    if call == 1 {
                        // Cancel is app-owned: keeping the window needs no Rune resolution API.
                        self.phase = Phase::SecondRequest;
                        println!("CLOSE CANCELLED AND STILL MAPPED");
                    } else {
                        assert_eq!(call, 2);
                        self.phase = Phase::Resolving;
                        println!("CLOSE APPROVED PROGRAMMATICALLY");
                        h.native_handle.close();
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    #[ignore = "requires the isolated headless close-consent driver"]
    #[allow(
        clippy::disallowed_methods,
        reason = "native test checks its isolation guard"
    )]
    fn native_close_request_consent() {
        assert_eq!(std::env::var("RUNE_CLOSE_NATIVE_TEST").as_deref(), Ok("1"));
        let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event()
            .with_wayland()
            .with_any_thread(true)
            .build()
            .unwrap();
        let proxy = event_loop.create_proxy();
        let pool = Rc::new(RefCell::new(Weak::new()));
        let (host, _) = make(proxy.clone(), pool.clone(), "consent");
        let mut probe = Probe {
            host: Some(host),
            proxy,
            pool,
            phase: Phase::Starting,
            deadline: std::time::Instant::now() + Duration::from_secs(30),
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            event_loop.run_app(&mut probe)
        }));
        // Also on a negative assertion: surfaces must die before winit's display does.
        probe.host.as_mut().unwrap().release();
        match result {
            Ok(result) => {
                result.unwrap();
                assert!(matches!(probe.phase, Phase::Done));
            }
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

/// Attention on a real Wayland window (`App::attention_wait`): run in an isolated compositor with
/// `RUNE_ATTENTION_NATIVE_TEST=1`.
#[cfg(target_os = "linux")]
mod attention_native {
    use super::*;
    use crate::platform::activation::{Activator, Attention, Counts, Wait};
    use crate::platform::attention::{State, step, Event};
    use winit::platform::wayland::EventLoopBuilderExtWayland;

    const LIMIT: Duration = Duration::from_secs(5);

    fn make(proxy: EventLoopProxy<WakeEvent>, pool: Rc<RefCell<Weak<Gpu>>>, key: &str, wait: bool) -> Host<()> {
        let app = crate::App::new(AppSpec::new("Attention probe", (300.0, 200.0), || (), |_, _| crate::text("Ready")));
        let app = if wait { app.attention_wait(LIMIT) } else { app };
        Host::new(WindowSpec::from_app(key, app), WindowHandle::new(key), Args::default(), proxy, 0, pool, true)
    }

    fn attention(h: &mut Host<()>) -> &mut Attention { &mut h.win.as_mut().expect("a shown window").native.attention }

    fn counts(h: &mut Host<()>) -> Option<Counts> {
        match attention(h) {
            State::Ready(a) | State::Unresponsive(a) => Some(a.counts()),
            _ => None,
        }
    }

    fn phase(h: &mut Host<()>) -> &'static str {
        match attention(h) {
            State::Unprobed => "unprobed",
            State::Unavailable => "unavailable",
            State::Ready(_) => "ready",
            State::Unresponsive(_) => "unresponsive",
        }
    }

    fn live_tokens(h: &mut Host<()>) -> usize {
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
    fn force_timeout(h: &mut Host<()>) {
        let State::Ready(a) = attention(h) else { panic!("force_timeout needs a ready window") };
        assert_eq!(a.attend(Instant::now()), Wait::TimedOut);
        let event: Event<Activator> = Event::TimedOut;
        step(attention(h), event);
        assert_eq!(phase(h), "unresponsive");
    }

    fn checks(el: &ActiveEventLoop, h: &mut Host<()>, proxy: &EventLoopProxy<WakeEvent>, pool: &Rc<RefCell<Weak<Gpu>>>) {
        let stale = || Instant::now() - LIMIT;

        // n1: an idle window's attention completes, twice, without waiting for winit.
        assert_eq!(phase(h), "unprobed");
        let took = timed(|| h.focus(el, Instant::now()));
        assert!(took < Duration::from_secs(1), "{took:?}");
        assert_eq!(phase(h), "ready");
        assert_eq!(counts(h), Some(Counts { exchanges: 1, activates: 1, syncs: 1 }));
        assert_eq!(live_tokens(h), 0, "the token is destroyed at done");
        h.focus(el, Instant::now());
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
        h.window_request(el, WindowRequest::Open(Some("forwarded".into())), Instant::now());
        assert_eq!(counts(h).unwrap(), Counts { activates: before.activates + 1, syncs: before.syncs + 1, ..before });
        force_timeout(h);
        let before = counts(h).unwrap();
        h.window_request(el, WindowRequest::Open(Some("after-timeout".into())), Instant::now());
        assert_eq!(counts(h).unwrap(), Counts { activates: before.activates + 1, ..before });
        println!("ATTENTION N3 PASS");

        // n2: the timed-out window's in-flight token goes with it; a new window starts over.
        assert!(phase(h) == "unresponsive" || phase(h) == "ready");
        h.hide(el);
        assert!(h.win.is_none(), "a Wayland hide destroys the window");
        h.show(el, Instant::now());
        assert_eq!(phase(h), "unprobed");
        h.focus(el, Instant::now());
        assert_eq!(phase(h), "ready");
        println!("ATTENTION N2 PASS");

        // n6, stale forwarded token on a fresh window: no bind at all.
        h.hide(el);
        h.show(el, Instant::now());
        h.window_request(el, WindowRequest::Open(Some("stale-unprobed".into())), stale());
        assert_eq!(phase(h), "unprobed", "a stale request never binds");
        println!("ATTENTION N6 UNPROBED PASS");

        // n4: without `attention_wait`, focus binds nothing and never waits.
        let mut plain = make(proxy.clone(), pool.clone(), "plain", false);
        plain.resume(el);
        let took = timed(|| plain.focus(el, Instant::now()));
        assert!(took < Duration::from_millis(100), "{took:?}");
        assert_eq!(phase(&mut plain), "unprobed");
        println!("ATTENTION N4 PASS");

        // n5: after a terminal GPU failure, attention neither reopens the window nor clears it.
        plain.hooks.gpu_failure = gpu_failure::Policy::Failed;
        plain.hide(el);
        plain.window_request(el, WindowRequest::Open(None), Instant::now());
        plain.focus(el, Instant::now());
        assert!(plain.win.is_none() && matches!(plain.hooks.gpu_failure, gpu_failure::Policy::Failed));
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
        host: Option<Host<()>>,
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
            }
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}
