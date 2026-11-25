use super::*;

/// From, event, to, and the effects the step asks for.
type Row = (State<()>, Event<()>, State<()>, Effects<Effect>);

fn stepped(mut state: State<()>, event: Event<()>) -> (State<()>, Effects<Effect>) {
    let effects = step(&mut state, event);
    (state, effects)
}

#[test]
fn every_transition_lands_where_the_table_says() {
    use Event::*;
    let none = Effects::none;
    let log = || Effects::one(Effect::LogTimeout);
    let rows: [Row; 18] = [
        (State::Unprobed, Bound(()), State::Ready(()), none()),
        (State::Unprobed, BindMissing, State::Unavailable, none()),
        (State::Unprobed, BindTimedOut(()), State::Unresponsive(()), log()),
        (State::Unprobed, Broken, State::Unavailable, none()),
        (State::Ready(()), Met, State::Ready(()), none()),
        (State::Ready(()), TimedOut, State::Unresponsive(()), log()),
        (State::Ready(()), Broken, State::Unavailable, none()),
        (State::Unresponsive(()), Answered, State::Ready(()), none()),
        (State::Unresponsive(()), AnsweredMissing, State::Unavailable, none()),
        (State::Unresponsive(()), Broken, State::Unavailable, none()),
        (State::Unresponsive(()), TimedOut, State::Unresponsive(()), none()),
        (State::Unavailable, Bound(()), State::Unavailable, none()),
        (State::Unavailable, Answered, State::Unavailable, none()),
        (State::Unavailable, Met, State::Unavailable, none()),
        (State::Unavailable, Reprobe, State::Unprobed, none()),
        (State::Unprobed, Reprobe, State::Unprobed, none()),
        (State::Ready(()), Reprobe, State::Ready(()), none()),
        (State::Unresponsive(()), Reprobe, State::Unresponsive(()), none()),
    ];
    for (from, event, to, effects) in rows {
        let label = format!("{from:?} + {event:?}");
        assert_eq!(stepped(from, event), (to, effects), "{label}");
    }
}

#[test]
fn each_answer_steps_an_unresponsive_window() {
    let after = |answer: Answer| {
        let mut state = State::Unresponsive(());
        if let Some(event) = answer.event() { step(&mut state, event); }
        state
    };
    assert_eq!(after(Answer::Pending), State::Unresponsive(()));
    assert_eq!(after(Answer::Ready), State::Ready(()));
    assert_eq!(after(Answer::Missing), State::Unavailable);
    assert_eq!(after(Answer::Broken), State::Unavailable);
}

#[test]
fn the_bind_ends_on_the_sync_not_on_the_global() {
    let mut missing = Registry::default();
    missing.on_global(1, "wl_compositor");
    missing.on_global(2, "xdg_activation_v2");
    missing.on_global(3, "zxdg_activation_v1");
    assert_eq!(missing.outcome(), None, "the list isn't over before the sync");
    missing.on_synced();
    assert_eq!(missing.outcome(), Some(Found::Missing));

    let mut found = Registry::default();
    found.on_global(7, INTERFACE);
    assert_eq!(found.outcome(), None, "seeing the global doesn't end the list");
    found.on_synced();
    assert_eq!(found.outcome(), Some(Found::Global(7)));
}

#[test]
fn a_withdrawn_global_is_never_bound() {
    let mut withdrawn = Registry::default();
    withdrawn.on_global(7, INTERFACE);
    withdrawn.on_global_remove(7);
    withdrawn.on_synced();
    assert_eq!(withdrawn.outcome(), Some(Found::Missing));

    // Withdrawn after the list ended, before a late bind finishes it (`answered`).
    let mut late = Registry::default();
    late.on_global(7, INTERFACE);
    late.on_synced();
    late.on_global_remove(7);
    assert_eq!(late.outcome(), Some(Found::Missing));

    let mut other = Registry::default();
    other.on_global(7, INTERFACE);
    other.on_global_remove(8);
    other.on_synced();
    assert_eq!(other.outcome(), Some(Found::Global(7)), "another global's removal changes nothing");
}

#[test]
fn a_request_waits_only_on_a_ready_window_while_it_is_fresh() {
    use Act::*;
    let (unprobed, unavailable, ready, unresponsive) =
        (State::<()>::Unprobed, State::<()>::Unavailable, State::Ready(()), State::Unresponsive(()));
    // (state, token, found, stale) -> act
    let rows: [(&State<()>, bool, bool, bool, Act); 16] = [
        (&ready, true, true, false, ActivateSynced),
        (&ready, false, true, false, Attend),
        // Stale: nothing waits, but a token is still handed over.
        (&ready, true, true, true, Activate),
        (&ready, false, true, true, Urgent),
        // Unresponsive (still, after the recovery check): never waits, whether stale or not.
        (&unresponsive, true, true, false, Activate),
        (&unresponsive, true, true, true, Activate),
        (&unresponsive, false, true, false, Urgent),
        (&unresponsive, false, true, true, Urgent),
        // A bind that ran out before the list came: no global bound to hand a token to.
        (&unresponsive, true, false, false, Urgent),
        (&unresponsive, true, false, true, Urgent),
        (&unavailable, true, false, false, Urgent),
        (&unavailable, false, false, false, Urgent),
        (&unavailable, true, false, true, Urgent),
        // Unprobed after `binds`: only a stale request, which never binds.
        (&unprobed, true, false, true, Urgent),
        (&unprobed, false, false, true, Urgent),
        (&unprobed, false, false, false, Urgent),
    ];
    for (state, token, found, stale, want) in rows {
        assert_eq!(act(state, token, found, stale), want, "{state:?} token={token} found={found} stale={stale}");
    }
}

#[test]
fn only_a_fresh_request_on_an_unprobed_window_binds() {
    assert!(binds(&State::<()>::Unprobed, false));
    assert!(!binds(&State::<()>::Unprobed, true), "a stale request never waits, so it never binds");
    for state in [State::Unavailable, State::Ready(()), State::Unresponsive(())] {
        assert!(!binds(&state, false), "{state:?}");
    }
}

#[test]
fn a_forwarded_token_activates_once_a_missing_global_appears() {
    let mut state = State::<u8>::Unprobed;
    assert_eq!(forward_with(&mut state, || Event::BindMissing), None, "no global yet");
    assert_eq!(state, State::Unavailable);
    // The compositor advertises the global later: the next forwarded token binds and activates.
    assert_eq!(forward_with(&mut state, || Event::Bound(7)).copied(), Some(7));
    assert_eq!(state, State::Ready(7));
    // A bound window activates with its binding and doesn't probe again.
    assert_eq!(forward_with(&mut state, || unreachable!("a bound window rebound")).copied(), Some(7));

    // A broken connection is probed again too, as the base did.
    let mut broken = State::<u8>::Unprobed;
    assert_eq!(forward_with(&mut broken, || Event::Broken), None);
    assert_eq!(forward_with(&mut broken, || Event::Bound(9)).copied(), Some(9));
}

#[test]
fn a_forwarded_token_probes_again_until_the_window_has_a_binding() {
    assert!(forward_binds(&State::<()>::Unprobed));
    assert!(forward_binds(&State::<()>::Unavailable), "a missing global is looked for again");
    assert!(!forward_binds(&State::Ready(())));
    // An app that doesn't wait never times out, so it never reaches `Unresponsive`.
    assert!(!forward_binds(&State::Unresponsive(())));
}

#[test]
fn a_request_is_bounded_from_when_it_was_made() {
    let limit = Duration::from_secs(5);
    let at = crate::time::now();
    let end = deadline(at, limit);
    assert_eq!(end, at + limit);
    // Handled 4.5 s late, it has half a second left, not a fresh five.
    let handled = at + Duration::from_millis(4500);
    assert_eq!(end.saturating_duration_since(handled), Duration::from_millis(500));
    // Handled at its deadline, it has nothing left: it doesn't wait.
    assert_eq!(end.saturating_duration_since(at + limit), Duration::ZERO);
}

#[test]
fn an_endless_limit_waits_a_day_and_never_overflows() {
    let at = crate::time::now();
    assert_eq!(deadline(at, Duration::MAX), at + LONGEST);
    assert_eq!(deadline(at, LONGEST + Duration::from_secs(1)), at + LONGEST);
    assert_eq!(deadline(at, LONGEST), at + LONGEST);
}
