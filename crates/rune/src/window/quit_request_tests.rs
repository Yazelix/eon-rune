use super::*;
use crate::QuitResponse;
use crate::quit::request;

#[derive(Default)]
struct Seen {
    requests: usize,
    notifications: usize,
    live_only: bool,
    quit_in_hook: bool,
}

fn consent(s: &mut Seen, tx: &mut Tx) -> QuitResponse {
    s.requests += 1;
    if s.live_only {
        tx.live_only();
    }
    if s.quit_in_hook {
        tx.quit();
    }
    QuitResponse::Defer
}

fn app() -> crate::App<Seen> {
    crate::App::new(AppSpec::new(
        "Quit consent",
        (100.0, 100.0),
        Seen::default,
        |s, _| crate::text(if s.requests == 0 { "Ready" } else { "Quit?" }).key("quit-prompt"),
    ))
    .on_quit_request(consent)
    .on_quit(|s, _| s.notifications += 1)
}

#[test]
fn headless_apps_do_not_register_and_window_specs_retain_the_callback() {
    assert!(!request::registered());
    let plain = crate::App::new(AppSpec::new(
        "default",
        (100.0, 100.0),
        || (),
        |_, _| crate::el(),
    ));
    assert!(plain.hooks.quit_request.is_none());
    let spec = WindowSpec::from_app("quit", app());
    assert!(spec.app.hooks.quit_request.is_some());
    let _rt = spec
        .app
        .headless_runtime((100.0, 100.0), crate::Env::new(false, 1.0));
    assert!(!request::registered());
}

#[test]
fn typed_host_delivery_defers_then_existing_quit_notifies_once() {
    let mut app = app();
    let on_quit = app.on_quit.unwrap();
    let hooks: HostHooks<Seen> = std::mem::take(&mut app.hooks).into();
    let mut rt = app.headless_runtime((100.0, 100.0), crate::Env::new(false, 1.0));
    let callback = hooks.quit_request.unwrap();
    assert_eq!(request::request(|| true), request::NativeReply::Later);
    assert!(request::dispatch([callback], |f| {
        assert!(rt.update_if(|s, tx| deliver_quit(f, s, tx)));
    }));
    assert_eq!(rt.state.requests, 1);
    assert_eq!(rt.state.notifications, 0, "on_quit is not consent");
    assert_eq!(rt.shell_request, None);
    assert_eq!(request::flush(), Some(false));
    rt.frame(0.0);
    assert_eq!(
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("quit-prompt"))
            .unwrap()
            .text
            .as_deref(),
        Some("Quit?")
    );
    rt.update(|s, tx| {
        s.quit_in_hook = true;
        tx.quit();
    });
    assert_eq!(rt.shell_request.take(), Some(Shell::Quit));
    assert_eq!(
        rt.state.requests, 1,
        "programmatic permit bypasses the hook"
    );
    // The existing quit lifecycle delivers notification once; exercise its real effects.
    let mut state = crate::quit::State::Running;
    for _ in 0..2 {
        let (next, effects) = crate::quit::step(
            state,
            crate::quit::Event::Start {
                ending: false,
                after: crate::quit::After::Exit,
            },
        );
        state = next;
        for effect in effects {
            if effect == crate::quit::Effect::RunHooks {
                rt.update(on_quit);
                state = crate::quit::step(state, crate::quit::Event::HooksRan).0;
            }
        }
    }
    assert_eq!(rt.state.notifications, 1);
    drop(hooks);
    assert!(!request::registered());
}

#[test]
fn live_only_and_explicit_quit_keep_their_transaction_meaning() {
    let mut app = app();
    let hooks: HostHooks<Seen> = std::mem::take(&mut app.hooks).into();
    let mut rt = app.headless_runtime((100.0, 100.0), crate::Env::new(false, 1.0));
    rt.state.live_only = true;
    rt.state.quit_in_hook = true;
    assert_eq!(request::request(|| true), request::NativeReply::Later);
    assert!(request::dispatch([hooks.quit_request.unwrap()], |f| {
        assert!(!rt.update_if(|s, tx| deliver_quit(f, s, tx)));
        assert_eq!(rt.shell_request.take(), Some(Shell::Quit));
        assert!(
            request::finish(),
            "the exit is answered through AppKit instead of exiting winit twice"
        );
    }));
    assert_eq!(
        request::flush(),
        Some(true),
        "explicit Quit overrides the callback's Defer"
    );
    assert_eq!(request::flush(), None);
}
