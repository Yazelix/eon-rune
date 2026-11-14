use super::*;

fn effects(state: &mut State, event: Event) -> Vec<Effect> {
    step(state, event).into_iter().collect()
}

#[test]
fn a_registered_request_waits_for_consent_instead_of_terminating_now() {
    let mut state = State::Idle;
    assert_eq!(
        effects(
            &mut state,
            Event::Request {
                registered: false,
                sent: false
            }
        ),
        [Effect::Native(NativeReply::Now)]
    );
    assert_eq!(state, State::Idle);
    assert_eq!(
        effects(
            &mut state,
            Event::Request {
                registered: true,
                sent: true
            }
        ),
        [Effect::Native(NativeReply::Later)]
    );
    assert_eq!(state, State::Queued);
    assert_eq!(
        effects(
            &mut state,
            Event::Request {
                registered: true,
                sent: true
            }
        ),
        [Effect::Native(NativeReply::Cancel)]
    );
    assert_eq!(effects(&mut state, Event::Begin), [Effect::Deliver]);
    assert_eq!(
        effects(
            &mut state,
            Event::Request {
                registered: true,
                sent: true
            }
        ),
        [Effect::Native(NativeReply::Cancel)]
    );
    assert_eq!(effects(&mut state, Event::Defer), [Effect::ScheduleReply]);
    assert!(
        effects(&mut state, Event::End).is_empty(),
        "a veto cannot be overwritten by the dispatch ending"
    );
    assert_eq!(
        effects(
            &mut state,
            Event::Request {
                registered: true,
                sent: true
            }
        ),
        [Effect::Native(NativeReply::Cancel)]
    );
    assert_eq!(effects(&mut state, Event::Replied), [Effect::Reply(false)]);
    assert!(effects(&mut state, Event::Replied).is_empty());
    assert_eq!(state, State::Idle);
}

#[test]
fn every_exit_resolves_pending_once_and_programmatic_quit_can_override_a_veto() {
    for start in [
        State::Queued,
        State::Asking,
        State::ReplyQueued(false),
        State::ReplyQueued(true),
    ] {
        let mut state = start;
        let scheduled = effects(&mut state, Event::Finish);
        assert_eq!(
            scheduled.len(),
            usize::from(matches!(start, State::Queued | State::Asking))
        );
        assert_eq!(state, State::ReplyQueued(true));
        assert!(effects(&mut state, Event::Finish).is_empty());
        assert_eq!(effects(&mut state, Event::Replied), [Effect::Reply(true)]);
        assert!(effects(&mut state, Event::Finish).is_empty());
        assert!(effects(&mut state, Event::Replied).is_empty());
    }
}

#[test]
fn delivery_failure_and_stray_events_do_not_leave_a_native_transaction() {
    let mut state = State::Idle;
    for event in [
        Event::Begin,
        Event::Defer,
        Event::End,
        Event::Finish,
        Event::Replied,
        Event::Blocked,
        Event::Returned,
    ] {
        assert!(effects(&mut state, event).is_empty());
    }
    assert_eq!(
        effects(
            &mut state,
            Event::Request {
                registered: true,
                sent: false
            }
        ),
        [Effect::Native(NativeReply::Now)]
    );
    assert_eq!(state, State::Idle);
    assert!(effects(&mut state, Event::Begin).is_empty());
    effects(
        &mut state,
        Event::Request {
            registered: true,
            sent: true,
        },
    );
    effects(&mut state, Event::Begin);
    assert_eq!(
        effects(
            &mut state,
            Event::Request {
                registered: true,
                sent: false
            }
        ),
        [Effect::Native(NativeReply::Cancel)]
    );
    assert_eq!(effects(&mut state, Event::End), [Effect::ScheduleReply]);
    assert_eq!(effects(&mut state, Event::Replied), [Effect::Reply(true)]);
}

#[test]
fn registration_and_dispatch_use_one_reply_and_restore_the_default_after_drop() {
    assert!(!registered());
    assert_eq!(
        request(|| panic!("no handler must not queue")),
        NativeReply::Now
    );
    let first = Registration::new();
    let second = Registration::new();
    assert_eq!(request(|| false), NativeReply::Now);
    assert_eq!(request(|| true), NativeReply::Later);
    assert_eq!(
        request(|| panic!("a duplicate must not queue")),
        NativeReply::Cancel
    );
    let mut seen = Vec::new();
    assert!(dispatch(0..3, |i| {
        seen.push(i);
        answer(if i == 1 {
            QuitResponse::Defer
        } else {
            QuitResponse::Quit
        });
    }));
    assert_eq!(seen, [0, 1], "stop at the first prompt");
    assert_eq!(flush(), Some(false));
    assert_eq!(flush(), None);
    assert!(!dispatch(0..1, |_| panic!("no pending request")));
    assert_eq!(request(|| true), NativeReply::Later);
    assert!(dispatch(0..2, |_| answer(QuitResponse::Quit)));
    assert_eq!(flush(), Some(true));
    assert_eq!(request(|| true), NativeReply::Later);
    drop(first);
    assert_eq!(flush(), None, "one registered host still owns consent");
    drop(second);
    assert_eq!(
        flush(),
        Some(true),
        "the last host going resolves the pending Quit"
    );
    assert_eq!(request(|| panic!("registration leaked")), NativeReply::Now);
}

#[test]
fn registrations_are_local_to_the_ui_thread() {
    let guard = Registration::new();
    assert!(registered());
    std::thread::spawn(|| {
        assert!(!registered());
        assert_eq!(
            request(|| panic!("a different UI thread")),
            NativeReply::Now
        );
    })
    .join()
    .unwrap();
    drop(guard);
}

#[test]
fn blocked_reply_rearms_once_and_keeps_explicit_quit() {
    for allow in [false, true] {
        let mut state = State::ReplyQueued(allow);
        assert!(effects(&mut state, Event::Blocked).is_empty());
        assert_eq!(state, State::ReplyHeld(allow));
        assert!(effects(&mut state, Event::Replied).is_empty());
        assert!(effects(&mut state, Event::Blocked).is_empty());
        assert_eq!(
            effects(
                &mut state,
                Event::Request {
                    registered: true,
                    sent: true
                }
            ),
            [Effect::Native(NativeReply::Cancel)]
        );
        assert!(effects(&mut state, Event::Finish).is_empty());
        assert_eq!(state, State::ReplyHeld(true));
        assert_eq!(
            effects(&mut state, Event::Returned),
            [Effect::ScheduleReply]
        );
        assert!(effects(&mut state, Event::Returned).is_empty());
        assert_eq!(effects(&mut state, Event::Replied), [Effect::Reply(true)]);
        assert!(effects(&mut state, Event::Replied).is_empty());
    }
}

#[test]
fn native_reply_waits_for_the_outermost_winit_handler() {
    for quit_while_held in [false, true] {
        let registration = Registration::new();
        assert_eq!(request(|| true), NativeReply::Later);
        assert!(dispatch([()], |_| answer(QuitResponse::Defer)));
        let outer = crate::quit::handling();
        let inner = crate::quit::handling();
        assert_eq!(
            flush(),
            None,
            "a common-mode callback inside a modal cannot reenter winit"
        );
        if quit_while_held {
            assert!(finish());
        }
        drop(inner);
        REQUESTS.with(|r| assert_eq!(r.borrow().state, State::ReplyHeld(quit_while_held)));
        assert_eq!(flush(), None, "nested callbacks leave the reply held");
        drop(outer);
        REQUESTS.with(|r| {
            assert_eq!(
                r.borrow().state,
                State::ReplyQueued(quit_while_held),
                "outer return queues completion without replying inside winit's borrow"
            )
        });
        let next_handler = crate::quit::handling();
        assert_eq!(
            flush(),
            None,
            "another handler can hold the rearmed callback again"
        );
        drop(next_handler);
        assert_eq!(flush(), Some(quit_while_held));
        assert_eq!(flush(), None);
        drop(registration);
    }
}
