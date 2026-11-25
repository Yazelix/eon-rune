use std::sync::Arc;

use super::app::{TypedWindowApp, WindowApp};
use crate::{App, AppSpec, QuitResponse, Tx, WindowSpec, el::Shell, quit::request};

#[derive(Default)]
struct Seen {
    requests: usize,
    notifications: usize,
    live_only: bool,
    quit_in_hook: bool,
}

fn consent(seen: &mut Seen, tx: &mut Tx) -> QuitResponse {
    seen.requests += 1;
    if seen.live_only { tx.live_only(); }
    if seen.quit_in_hook { tx.quit(); }
    QuitResponse::Defer
}

fn app() -> App<Seen> {
    App::new(AppSpec::new("Quit consent", (100.0, 100.0), Seen::default, |seen, _| {
        crate::text(if seen.requests == 0 { "Ready" } else { "Quit?" }).key("quit-prompt")
    }))
    .on_quit_request(consent)
    .on_quit(|seen, _| seen.notifications += 1)
}

#[test]
fn headless_apps_do_not_register_and_window_specs_retain_the_callback() {
    assert!(!request::registered());
    let plain = App::new(AppSpec::new("Default", (100.0, 100.0), || (), |_, _| crate::el()));
    assert!(plain.on_quit_request.is_none());
    let spec = WindowSpec::from_app("quit", app());
    assert!(spec.app.on_quit_request.is_some());
    let _runtime = spec.app.headless_runtime((100.0, 100.0), crate::Env::new(false, 1.0));
    assert!(!request::registered());
}

#[test]
fn typed_host_delivery_defers_then_existing_quit_notifies_once() {
    let mut host = TypedWindowApp::from_app(app(), Arc::new(|| {}));
    host.rt.frame(0.0);
    assert_eq!(request::request(|| true), request::NativeReply::Later);
    assert!(request::dispatch([&mut host], |host| {
        let (response, stale) = host.run_on_quit_request().unwrap();
        assert!(stale);
        request::answer(response);
    }));
    assert_eq!(host.rt.state.requests, 1);
    assert_eq!(host.rt.state.notifications, 0);
    assert_eq!(host.rt().shell_request, None);
    assert_eq!(request::flush(), Some(false));
    host.rt.frame(0.0);
    assert_eq!(host.rt.elements().into_iter().find(|e| e.key.as_deref() == Some("quit-prompt"))
        .unwrap().text.as_deref(), Some("Quit?"));

    host.rt.update(|_, tx| tx.quit());
    assert_eq!(host.rt_mut().shell_request.take(), Some(Shell::Quit));
    assert_eq!(host.rt.state.requests, 1);
    let mut state = crate::quit::State::Running;
    for _ in 0..2 {
        let (next, effects) = crate::quit::step(state, crate::quit::Event::Start {
            ending: false, after: crate::quit::After::Exit,
        });
        state = next;
        for effect in effects {
            if effect == crate::quit::Effect::RunHooks {
                host.run_on_quit();
                state = crate::quit::step(state, crate::quit::Event::HooksRan).0;
            }
        }
    }
    assert_eq!(host.rt.state.notifications, 1);
    drop(host);
    assert!(!request::registered());
}

#[test]
fn live_only_and_explicit_quit_keep_their_transaction_meaning() {
    let mut host = TypedWindowApp::from_app(app(), Arc::new(|| {}));
    host.rt.state.live_only = true;
    host.rt.state.quit_in_hook = true;
    host.rt.frame(0.0);
    assert_eq!(request::request(|| true), request::NativeReply::Later);
    assert!(request::dispatch([&mut host], |host| {
        let (response, stale) = host.run_on_quit_request().unwrap();
        assert!(!stale);
        request::answer(response);
        assert_eq!(host.rt_mut().shell_request.take(), Some(Shell::Quit));
        assert!(request::finish());
    }));
    assert_eq!(request::flush(), Some(true), "explicit Quit overrides Defer");
    assert_eq!(request::flush(), None);
}
