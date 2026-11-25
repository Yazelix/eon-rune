//! Each place a window decides to skip a frame tells the app why, inside the call that decides
//! it, and a window whose GPU failed under `App::terminal_gpu_loss` reports nothing.

use super::{Host, tests::test_host};
use crate::{WindowOptions, WindowSpec, time::now, window::{WakeEvent, surface::{Backoff, Missed, TIMEOUT_RETRY}}};
use std::rc::Rc;
use winit::{application::ApplicationHandler, event::WindowEvent, event_loop::{ActiveEventLoop, EventLoop}, window::WindowId};
use crate::{FrameOutcome, FrameSkip};
use std::cell::RefCell;
use winit::platform::wayland::{EventLoopBuilderExtWayland, WindowExtWayland};

type Log = Rc<RefCell<Vec<(FrameOutcome, Option<FrameSkip>)>>>;
/// A decision point a GPU failure must stop before it reports.
type Decide = fn(&mut Host);

fn probe(event_loop: &EventLoop<WakeEvent>, terminal: bool, log: &Log) -> Host {
    let sink = log.clone();
    let app = crate::App::new(crate::AppSpec::new("Skip reason probe", (200.0, 100.0), || (), |_, _| crate::el()))
        .keep_running()
        .window_options(WindowOptions { borderless: true, ..Default::default() })
        .on_frame_encoded(move |f| sink.borrow_mut().push((f.outcome(), f.skipped())));
    let app = if terminal { app.terminal_gpu_loss() } else { app };
    test_host(WindowSpec::from_app("skip-probe", app), event_loop, true)
}

/// The report `step` added, which must be exactly one.
fn reported(log: &Log, step: impl FnOnce()) -> (FrameOutcome, Option<FrameSkip>) {
    let before = log.borrow().len();
    step();
    let log = log.borrow();
    assert_eq!(log.len(), before + 1, "one report inside the call: {:?}", &log[before..]);
    log[before]
}

fn checks(host: &mut Host, log: &Log, terminal: &mut [(Host, Log)], el: &ActiveEventLoop) {
    host.resume(el);
    if !host.life.shown() { host.show(el, now()); }
    host.redraw();
    let window = &host.win.as_ref().unwrap().window;
    assert!(!window.has_client_side_frame(), "a borderless Wayland window has no frame");
    assert_eq!(window.content_corner_radius(), 0.0);
    assert!(host.life.uncovered(), "the probe window is shown and seen");
    assert!(log.borrow().contains(&(FrameOutcome::Drawn, None)), "a drawn frame has no reason: {:?}", log.borrow());
    assert!(log.borrow().iter().all(|(outcome, why)| why.is_some() == (*outcome == FrameOutcome::Skipped)));

    // The window's own occlusion event covers it, and its next frame is covered. This runs
    // before any acquire miss marks the surface covered, so only the native event can.
    let Some(id) = host.win.as_ref().map(|w| w.window.id()) else { panic!("the probe has a window") };
    host.handle_window_event(el, id, WindowEvent::Occluded(true));
    assert!(!host.life.uncovered());
    assert_eq!(reported(log, || host.redraw()), (FrameOutcome::Skipped, Some(FrameSkip::Covered)));
    host.handle_window_event(el, id, WindowEvent::Occluded(false));
    assert!(host.life.uncovered(), "uncovered again");

    // A shown window's acquire misses.
    for (missed, why) in [(Missed::Timeout, FrameSkip::Timeout), (Missed::Outdated, FrameSkip::Surface),
        (Missed::Unconfigured, FrameSkip::Surface), (Missed::Occluded, FrameSkip::Covered)] {
        assert_eq!(reported(log, || host.missed_frame(missed, false)), (FrameOutcome::Skipped, Some(why)), "{missed:?}");
    }
    // A hidden launcher's acquire misses (an unconfigured surface is caught before this arm).
    // Each one also leaves the cached frame stale and waits out the launcher's own backoff,
    // which doubles from `TIMEOUT_RETRY` on every miss.
    let Some(l) = host.win.as_mut() else { panic!("the probe has a window") };
    l.backoff = Backoff::default();
    assert!(l.first_frame.deadline().is_none(), "no first frame prepares, so the window backs off itself");
    let mut wait = TIMEOUT_RETRY;
    for (missed, why) in [(Missed::Timeout, FrameSkip::Timeout), (Missed::Outdated, FrameSkip::Surface), (Missed::Occluded, FrameSkip::Covered)] {
        let before = now();
        assert_eq!(reported(log, || host.hidden_missed(missed)), (FrameOutcome::Skipped, Some(why)), "{missed:?}");
        let Some(l) = host.win.as_ref() else { panic!("the probe has a window") };
        assert!(!l.first_frame.ready(), "{missed:?}: the cached frame is stale");
        let Some(at) = l.backoff.deadline() else { panic!("{missed:?}: the launcher backs off") };
        assert!(at >= before + wait, "{missed:?}: waits at least {wait:?}");
        wait *= 2;
    }

    // Under the terminal policy a recorded GPU failure stops each decision point before it
    // reports: one fresh window per call, so each call meets the failure with its window still
    // there.
    let deciders: [(&str, Decide); 3] = [
        ("hidden_missed", |h| h.hidden_missed(Missed::Timeout)),
        ("missed_frame", |h| h.missed_frame(Missed::Timeout, false)),
        ("covered_frame", |h| h.covered_frame()),
    ];
    for ((name, decide), (failed, log)) in deciders.into_iter().zip(terminal) {
        failed.resume(el);
        if !failed.life.shown() { failed.show(el, now()); }
        assert!(failed.win.is_some() && failed.life.shown(), "{name}: a shown window");
        let before = log.borrow().len();
        let Some((gpu, _)) = failed.gpu.as_ref() else { panic!("{name}: the probe has a GPU") };
        gpu.failure.record("injected failure before a skip".into());
        decide(failed);
        assert_eq!(log.borrow().len(), before, "{name} reported after the GPU failed: {:?}", log.borrow());
        assert!(failed.win.is_none(), "{name}: the failure gave the window up");
    }
}

#[test]
#[ignore = "requires a private headless Wayland session, a Vulkan adapter and RUNE_SKIP_REASON_NATIVE=1"]
#[allow(clippy::disallowed_methods, reason = "explicit isolation gate for native test")]
fn native_frame_skips_report_their_reason() {
    assert_eq!(std::env::var("RUNE_SKIP_REASON_NATIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("RUNE_CPU_FIRST").as_deref(), Ok("0"));
    assert!(std::env::var_os("DISPLAY").is_none());
    struct Probe { host: Host, log: Log, terminal: Vec<(Host, Log)>, ran: bool }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            checks(&mut self.host, &self.log, &mut self.terminal, el);
            self.ran = true;
            el.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let mut builder = EventLoop::<WakeEvent>::with_user_event();
    builder.with_wayland().with_any_thread(true);
    let event_loop = builder.build().unwrap();
    let log = Log::default();
    let host = probe(&event_loop, false, &log);
    let terminal = (0..3).map(|_| {
        let log = Log::default();
        (probe(&event_loop, true, &log), log)
    }).collect();
    let mut probe = Probe { host, log, terminal, ran: false };
    event_loop.run_app(&mut probe).unwrap();
    assert!(probe.ran);
    println!("SKIP REASON PASS");
}
