use super::*;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceId, MouseScrollDelta, Touch};

#[test]
fn lifecycle_and_modifier_events_cannot_be_consumed() {
    for event in [
        WindowEvent::CloseRequested,
        WindowEvent::Destroyed,
        WindowEvent::Resized(PhysicalSize::new(0, 300)),
        WindowEvent::Focused(false),
        WindowEvent::Occluded(true),
        WindowEvent::RedrawRequested,
        WindowEvent::ModifiersChanged(ModifiersState::SHIFT.into()),
    ] {
        assert!(!consumable(&event), "{event:?}");
    }
    let app = crate::App::new(AppSpec::new("hooks", (20.0, 20.0), || (), |_, _| crate::el()));
    assert!(app.hooks.window_event.is_none());
    let spec = WindowSpec::from_app("second", app.on_window_event(|_, _, _, _| true));
    assert!(spec.app.hooks.window_event.is_some());
}

fn inputs() -> Vec<WindowEvent> {
    let device_id = DeviceId::dummy();
    let mut events = vec![WindowEvent::CursorMoved { device_id, position: PhysicalPosition::new(17.25, 31.5) }, WindowEvent::CursorLeft { device_id }];
    for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle, MouseButton::Back, MouseButton::Forward, MouseButton::Other(17)] {
        events.push(WindowEvent::MouseInput { device_id, state: ElementState::Pressed, button });
    }
    for button in [MouseButton::Right, MouseButton::Left, MouseButton::Other(17)] {
        events.push(WindowEvent::MouseInput { device_id, state: ElementState::Released, button });
    }
    for phase in [TouchPhase::Started, TouchPhase::Moved, TouchPhase::Ended, TouchPhase::Cancelled] {
        for delta in [MouseScrollDelta::LineDelta(2.5, -3.25), MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.123456789, -9.75))] {
            events.push(WindowEvent::MouseWheel { device_id, delta, phase });
        }
        events.push(WindowEvent::Touch(Touch { device_id, phase, location: PhysicalPosition::new(11.125, 22.25), force: None, id: 0xffff_ffff_0000_0001 }));
    }
    events
}

#[test]
fn every_native_pointer_wheel_and_touch_phase_is_consumable() {
    for event in inputs() {
        assert!(consumable(&event), "{event:?}");
    }
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct Seen {
    events: Vec<WindowEvent>,
    address: usize,
    decline: bool,
    logical: Vec<crate::Key>,
    pasted: usize,
    repeats: usize,
    unidentified: usize,
    numpad: usize,
    mods: crate::Mods,
    time: Option<Instant>,
    focus: Vec<bool>,
}

#[cfg(target_os = "linux")]
fn record(s: &mut Seen, window: &Window, event: &WindowEvent, tx: &mut Tx) -> bool {
    assert!(window.inner_size().width > 0);
    s.address = std::ptr::from_ref(event) as usize;
    s.events.push(event.clone());
    s.mods = tx.mods();
    s.time = tx.input_time();
    tx.live_only();
    if let WindowEvent::KeyboardInput { event, .. } = event {
        use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        if matches!(event.physical_key, PhysicalKey::Unidentified(_)) {
            s.unidentified += 1;
            return true;
        }
        if event.physical_key == PhysicalKey::Code(KeyCode::Numpad1) {
            assert_eq!(event.location, winit::keyboard::KeyLocation::Numpad);
            s.numpad += 1;
            return true;
        }
        if event.logical_key == Key::Named(NamedKey::Paste) {
            assert_eq!(event.physical_key, PhysicalKey::Code(KeyCode::KeyV));
            assert_eq!(event.location, winit::keyboard::KeyLocation::Standard);
            assert_eq!(event.key_without_modifiers(), Key::Named(NamedKey::Paste));
            assert!(tx.input_time().is_some());
            let _ = (event.text.as_deref(), event.text_with_all_modifiers(), event.repeat);
            if event.repeat {
                s.repeats += 1;
            }
            if event.state == ElementState::Pressed && !event.repeat {
                s.pasted += 1;
            }
            return true;
        }
        if event.physical_key == PhysicalKey::Code(KeyCode::F12) {
            tx.close_window();
            return false;
        }
        return false;
    }
    !s.decline
}

// Run only inside the private Wayland driver, never against the user's display.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires isolated Wayland and the fixed-keymap injector"]
#[allow(clippy::disallowed_methods, reason = "native test checks its isolation guard at startup")]
fn native_window_hook_dispatch() {
    assert_eq!(std::env::var("RUNE_NATIVE_HOOK_TEST").as_deref(), Ok("1"));
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    struct Probe {
        host: Option<Host<Seen>>,
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let host = self.host.as_mut().unwrap();
            host.resume(el);
            let event = WindowEvent::MouseWheel { device_id: DeviceId::dummy(), delta: MouseScrollDelta::LineDelta(1.0, 7.0), phase: TouchPhase::Cancelled };
            let window_id = host.win.as_ref().unwrap().window.id();
            let original = std::ptr::from_ref(&event) as usize;
            let at = Instant::now();
            let scope = crate::input::time::scope(Some(at));
            host.mods = ModifiersState::SHIFT | ModifiersState::CONTROL;
            assert!(
                !{
                    let hook = host.hooks.window_event.take();
                    let result = host.before_window_event(el, &event);
                    host.hooks.window_event = hook;
                    result
                },
                "absent hook leaves fallback intact"
            );
            assert!(host.before_window_event(el, &event), "live_only still consumes");
            assert_eq!(host.rt.state.address, original, "the original event is borrowed");
            assert_eq!(host.rt.state.events.last(), Some(&event));
            assert_eq!(host.rt.state.mods, map_mods(host.mods));
            assert_eq!(host.rt.state.time, Some(at));
            drop(scope);
            let events = inputs();
            host.rt.state.events.clear();
            for event in &events {
                host.win.as_mut().unwrap().timer_frame.asked_quiet(Instant::now());
                host.handle_window_event(el, window_id, event.clone());
                assert!(!host.win.as_ref().unwrap().timer_frame.quiet(), "consumed input clears quiet-timer classification");
            }
            assert_eq!(host.rt.state.events, events, "original axes, units, phases, IDs and order");
            assert!(host.rt.pointer().is_none(), "consumed motion never reaches widget capture");
            assert!(!host.rt.touching(), "consumed touch never starts toolkit pan");
            host.rt.state.decline = true;
            host.handle_window_event(el, window_id, WindowEvent::CursorMoved { device_id: DeviceId::dummy(), position: PhysicalPosition::new(50.0, 60.0) });
            assert_eq!(host.rt.pointer(), Some((50.0 / host.rt.input_scale(), 60.0 / host.rt.input_scale())));
            host.handle_window_event(el, window_id, WindowEvent::ModifiersChanged(ModifiersState::SHIFT.into()));
            assert!(host.mods.shift_key(), "declining modifiers keep native bookkeeping");
            host.rt.state.decline = false;
            host.handle_window_event(el, window_id, WindowEvent::ModifiersChanged(ModifiersState::empty().into()));
            assert!(host.mods.is_empty(), "taking modifiers cannot suppress bookkeeping");
        }
        fn about_to_wait(&mut self, el: &ActiveEventLoop) {
            self.host.as_mut().unwrap().about_to_wait(el);
        }
        fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            let host = self.host.as_mut().unwrap();
            let _scope = crate::input::time::arrived(&event);
            host.handle_window_event(el, id, event);
            host.rt.frame(0.0);
            if host.life.finished() {
                assert_eq!(host.rt.state.pasted, 1, "remapped paste delivered once");
                assert!(host.rt.state.repeats > 0, "native repeats are preserved");
                assert_eq!(host.rt.state.unidentified, 2, "unidentified physical press and release are retained");
                assert_eq!(host.rt.state.numpad, 2, "numpad location is preserved");
                assert_eq!(host.rt.state.logical, vec![crate::Key::Text("a".into()), crate::Key::Text("'".into())]);
                assert!(host.rt.state.focus.contains(&true), "non-consumable focus reached Rune");
                println!("NATIVE HOOK PASS");
                el.exit();
            }
        }
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event().with_wayland().with_any_thread(true).build().unwrap();
    let mut spec = AppSpec::new("Native hook probe", (300.0, 200.0), Seen::default, |_, _| crate::el());
    spec.on_key = Some(|s, event, _| {
        s.logical.push(event.key.clone());
        true
    });
    let app = crate::App::new(spec)
        .on_window_event(record)
        .on_focus(|s, focus, _| s.focus.push(focus))
        .on_ready(|_| println!("NATIVE HOOK READY"));
    let host = Host::new(
        WindowSpec::from_app("probe", app),
        WindowHandle::new("probe"),
        Args::default(),
        event_loop.create_proxy(),
        0,
        Rc::new(RefCell::new(Weak::new())),
        true,
    );
    event_loop.run_app(&mut Probe { host: Some(host) }).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires an isolated Wayland session"]
#[allow(clippy::disallowed_methods, reason = "native test checks its isolation guard at startup")]
fn native_hook_close_preserves_focus_loss() {
    assert_eq!(std::env::var("RUNE_NATIVE_HOOK_TEST").as_deref(), Ok("1"));
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    struct Probe {
        host: Host<Seen>,
    }
    impl ApplicationHandler<WakeEvent> for Probe {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let host = &mut self.host;
            host.resume(el);
            let id = host.win.as_ref().unwrap().window.id();
            host.rt.window_focus(true);
            host.handle_window_event(el, id, WindowEvent::Focused(false));
            assert!(host.life.finished() && host.win.is_some(), "the hook closed a retained native window");
            assert!(!host.rt.has_window_focus(), "hook close must not swallow mandatory focus loss");
            assert_eq!(host.rt.state.focus.last(), Some(&false));
            el.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = winit::event_loop::EventLoop::<WakeEvent>::with_user_event().with_wayland().with_any_thread(true).build().unwrap();
    let spec = AppSpec::new("Focus cleanup probe", (300.0, 200.0), Seen::default, |_, _| crate::el());
    let app = crate::App::new(spec).on_focus(|s, focused, _| s.focus.push(focused)).on_window_event(|_, _, event, tx| {
        if matches!(event, WindowEvent::Focused(false)) {
            tx.close_window();
        }
        false
    });
    let host = Host::new(
        WindowSpec::from_app("focus", app),
        WindowHandle::new("focus"),
        Args::default(),
        event_loop.create_proxy(),
        0,
        Rc::new(RefCell::new(Weak::new())),
        true,
    );
    event_loop.run_app(&mut Probe { host }).unwrap();
}
