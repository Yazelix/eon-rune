//! Native application Quit consent; AppKit alone starts a pending request.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use crate::QuitResponse;
use crate::machine::Effects;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Idle,
    Queued,
    Asking,
    ReplyQueued(bool),
    ReplyHeld(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeReply {
    Cancel = 0,
    Now = 1,
    Later = 2,
}

#[derive(Clone, Copy, Debug)]
enum Event {
    Request { registered: bool, sent: bool },
    Begin,
    Defer,
    End,
    Finish,
    Replied,
    Blocked,
    Returned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    Native(NativeReply),
    Deliver,
    ScheduleReply,
    Reply(bool),
}

fn step(state: &mut State, event: Event) -> Effects<Effect> {
    use {Effect::*, Event::*, State::*};
    let (next, effects) = match (*state, event) {
        (
            Idle,
            Request {
                registered: true,
                sent: true,
            },
        ) => (Queued, Effects::one(Native(NativeReply::Later))),
        (Idle, Request { .. }) => (Idle, Effects::one(Native(NativeReply::Now))),
        (_, Request { .. }) => (*state, Effects::one(Native(NativeReply::Cancel))),
        (Queued, Begin) => (Asking, Effects::one(Deliver)),
        (Asking, Defer) => (ReplyQueued(false), Effects::one(ScheduleReply)),
        (Asking, End) | (Queued | Asking, Finish) => {
            (ReplyQueued(true), Effects::one(ScheduleReply))
        }
        (ReplyQueued(_), Finish) => (ReplyQueued(true), Effects::none()),
        (ReplyHeld(_), Finish) => (ReplyHeld(true), Effects::none()),
        (ReplyQueued(allow), Blocked) => (ReplyHeld(allow), Effects::none()),
        (ReplyHeld(allow), Returned) => (ReplyQueued(allow), Effects::one(ScheduleReply)),
        (ReplyQueued(allow), Replied) => (Idle, Effects::one(Reply(allow))),
        _ => (*state, Effects::none()),
    };
    *state = next;
    effects
}

#[derive(Default)]
struct Requests {
    state: State,
    handlers: usize,
}

thread_local! {
    static REQUESTS: RefCell<Requests> = RefCell::new(Requests::default());
}

fn send(event: Event) -> Effects<Effect> {
    REQUESTS.with(|r| step(&mut r.borrow_mut().state, event))
}

fn apply(effects: Effects<Effect>) {
    for effect in effects {
        if effect == Effect::ScheduleReply {
            #[cfg(target_os = "macos")]
            super::mac::schedule_reply();
        }
    }
}

pub(crate) fn registered() -> bool {
    REQUESTS.with(|r| r.borrow().handlers != 0)
}

pub(crate) fn request(queue: impl FnOnce() -> bool) -> NativeReply {
    let registered = registered();
    let idle = REQUESTS.with(|r| r.borrow().state == State::Idle);
    let sent = registered && idle && queue();
    send(Event::Request { registered, sent })
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Native(reply) => Some(reply),
            _ => None,
        })
        .unwrap_or(NativeReply::Cancel)
}

fn begin() -> bool {
    send(Event::Begin).into_iter().any(|e| e == Effect::Deliver)
}
pub(crate) fn asking() -> bool {
    REQUESTS.with(|r| r.borrow().state == State::Asking)
}
pub(crate) fn dispatch<T>(hosts: impl IntoIterator<Item = T>, mut ask: impl FnMut(T)) -> bool {
    if !begin() {
        return false;
    }
    for host in hosts {
        if !asking() {
            break;
        }
        ask(host);
    }
    end();
    true
}
pub(crate) fn answer(response: QuitResponse) {
    if response == QuitResponse::Defer {
        apply(send(Event::Defer));
    }
}
fn end() {
    apply(send(Event::End));
}

/// True when AppKit will end the loop; the caller must not exit it a second time.
pub(crate) fn finish() -> bool {
    let pending = REQUESTS.with(|r| r.borrow().state != State::Idle);
    apply(send(Event::Finish));
    pending
}

pub(crate) fn exit(event_loop: &winit::event_loop::ActiveEventLoop) {
    if !finish() {
        event_loop.exit();
    }
}

/// Hold completion inside a handler: replying true can synchronously deliver LoopExiting.
pub(crate) fn flush() -> Option<bool> {
    if super::loop_state() == super::Loop::Handling {
        send(Event::Blocked);
        return None;
    }
    send(Event::Replied).into_iter().find_map(|effect| {
        let Effect::Reply(allow) = effect else {
            return None;
        };
        #[cfg(target_os = "macos")]
        super::mac::reply(allow);
        Some(allow)
    })
}

pub(super) fn handler_returned() {
    apply(send(Event::Returned));
}

pub(crate) struct Registration(PhantomData<Rc<()>>);

impl Registration {
    pub(crate) fn new() -> Self {
        REQUESTS.with(|r| r.borrow_mut().handlers += 1);
        #[cfg(target_os = "macos")]
        super::mac::warn_if_unavailable();
        Self(PhantomData)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        REQUESTS.with(|r| r.borrow_mut().handlers -= 1);
        if !registered() {
            finish();
        }
    }
}

#[cfg(test)]
test_group! { native =>
#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
}
