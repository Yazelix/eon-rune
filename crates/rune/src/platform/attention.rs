//! Whether a Wayland window's attention requests wait for the compositor (`App::attention_wait`),
//! kept out of `platform/activation.rs` (which only builds on unix desktops) so its tests run on
//! every host: the window's binding state, what the registry said, and each request's deadline.

use crate::machine::Effects;
use crate::time::Instant;
use std::time::Duration;

/// One native window's binding to `xdg_activation_v1`, generic over the binding so tests use
/// `()`. A new native window (a Wayland show after a hide) starts again at `Unprobed`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) enum State<B> {
    /// Not bound yet: the next request that may wait binds.
    #[default]
    Unprobed,
    /// No global, no Wayland handles, or a broken connection: never waits again. Only a
    /// forwarded token of an app that doesn't wait probes again (`Event::Reprobe`).
    Unavailable,
    /// Bound, and the compositor answered the last wait.
    Ready(B),
    /// A wait ran out. Later requests don't wait until every reply it was owed arrives
    /// (`Answer::Ready`); the binding stays, possibly half-made (a bind that ran out).
    Unresponsive(B),
}

/// What a bind, an exchange or the recovery check reported.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Event<B> {
    Bound(B),
    BindMissing,
    BindTimedOut(B),
    Met,
    TimedOut,
    Broken,
    Answered,
    AnsweredMissing,
    /// An app that doesn't wait probes an `Unavailable` window again on each forwarded token.
    Reprobe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// One line: the compositor didn't confirm attention in time.
    LogTimeout,
}

pub(crate) fn step<B>(state: &mut State<B>, event: Event<B>) -> Effects<Effect> {
    match (std::mem::take(state), event) {
        (State::Unavailable, Event::Reprobe) => *state = State::Unprobed,
        (State::Unavailable, _) | (_, Event::Broken) => *state = State::Unavailable,
        (State::Unprobed, Event::Bound(b)) => *state = State::Ready(b),
        (State::Unprobed, Event::BindMissing) => *state = State::Unavailable,
        (State::Unprobed, Event::BindTimedOut(b)) => {
            *state = State::Unresponsive(b);
            return Effects::one(Effect::LogTimeout);
        }
        (State::Ready(b), Event::TimedOut) => {
            *state = State::Unresponsive(b);
            return Effects::one(Effect::LogTimeout);
        }
        (State::Unresponsive(b), Event::Answered) => *state = State::Ready(b),
        (State::Unresponsive(_), Event::AnsweredMissing) => *state = State::Unavailable,
        // `Met` keeps `Ready`; an event that doesn't belong to the state changes nothing.
        (other, _) => *state = other,
    }
    Effects::none()
}

/// The recovery check's result for an `Unresponsive` window, and the event it steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// A reply it was owed hasn't arrived.
    Pending,
    /// Every reply arrived and the global is bound.
    Ready,
    /// A late bind finished without the global.
    Missing,
    Broken,
}

impl Answer {
    pub(crate) fn event<B>(self) -> Option<Event<B>> {
        match self {
            Answer::Pending => None,
            Answer::Ready => Some(Event::Answered),
            Answer::Missing => Some(Event::AnsweredMissing),
            Answer::Broken => Some(Event::Broken),
        }
    }
}

pub(crate) const INTERFACE: &str = "xdg_activation_v1";

/// What the registry has said: the global's name if it was advertised, and whether the sync sent
/// after `get_registry` came back, which ends the global list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Registry {
    global: Option<u32>,
    synced: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    Global(u32),
    Missing,
}

impl Registry {
    pub(crate) fn on_global(&mut self, name: u32, interface: &str) {
        if interface == INTERFACE { self.global = Some(name); }
    }

    /// A withdrawn global must not be bound.
    pub(crate) fn on_global_remove(&mut self, name: u32) {
        if self.global == Some(name) { self.global = None; }
    }

    pub(crate) fn on_synced(&mut self) { self.synced = true; }

    /// Known once the sync came back, whether or not the global was advertised.
    pub(crate) fn outcome(&self) -> Option<Found> {
        self.synced.then(|| self.global.map_or(Found::Missing, Found::Global))
    }
}

/// What a request does once any bind is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    /// Get a token, activate with it, and wait for the compositor.
    Attend,
    /// Activate with the forwarded token and wait for the compositor.
    ActivateSynced,
    /// Hand the forwarded token over without waiting.
    Activate,
    /// winit's own request: the compositor may only mark the window urgent.
    Urgent,
}

/// Whether a request of an app that waits binds first: only an unprobed window, and only while
/// the request may still wait (`stale` is false).
pub(crate) fn binds<B>(state: &State<B>, stale: bool) -> bool { matches!(state, State::Unprobed) && !stale }

/// Whether a forwarded token of an app that doesn't wait binds first: until the window has a
/// binding, every forwarded token probes again.
pub(crate) fn forward_binds<B>(state: &State<B>) -> bool { matches!(state, State::Unprobed | State::Unavailable) }

/// A forwarded token of an app that doesn't wait: binds with `bind` while the window has no
/// binding (an `Unavailable` window probes again), and returns the binding to activate with.
/// Its bind has no deadline, so it never times out and has no effect to carry.
pub(crate) fn forward_with<B>(state: &mut State<B>, bind: impl FnOnce() -> Event<B>) -> Option<&mut B> {
    if forward_binds(state) {
        step(state, Event::Reprobe);
        let effects = step(state, bind());
        debug_assert!(effects.is_empty(), "a bind without a deadline never times out");
    }
    match state {
        State::Ready(b) => Some(b),
        _ => None,
    }
}

/// What a request of an app that waits does once any bind and recovery are over: a `Ready`
/// window waits unless the request is stale; a forwarded token is still handed over without
/// waiting while the global is bound (`found`); anything else asks winit for urgency.
pub(crate) fn act<B>(state: &State<B>, token: bool, found: bool, stale: bool) -> Act {
    match state {
        State::Ready(_) if !stale && token => Act::ActivateSynced,
        State::Ready(_) if !stale => Act::Attend,
        State::Ready(_) | State::Unresponsive(_) if token && found => Act::Activate,
        _ => Act::Urgent,
    }
}

/// The longest wait a request gets: a larger `App::attention_wait` limit counts as this.
pub(crate) const LONGEST: Duration = Duration::from_secs(24 * 60 * 60);

/// A request's deadline: `limit` (at most `LONGEST`) after it was made, however late it is
/// handled.
pub(crate) fn deadline(at: Instant, limit: Duration) -> Instant { at + limit.min(LONGEST) }

#[cfg(test)]
test_group! { core =>
#[cfg(test)]
#[path = "attention_tests.rs"]
mod tests;
}
