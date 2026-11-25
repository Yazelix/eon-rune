//! Wayland window activation on winit's own display connection, in a private queue.
//!
//! winit 0.30 uses a forwarded XDG activation token only while creating a window
//! (`focus_window` is a no-op on Wayland), so [`forward`] binds `xdg_activation_v1` and
//! activates winit's surface with it. With `App::attention_wait`, [`request`] also waits until the
//! compositor has processed each attention request (token, activate, then a later display sync),
//! within the request's deadline (`attention::deadline`): a fast exit after the request would
//! otherwise drop it before the compositor read it. The wait reads the socket itself, so it never
//! depends on winit's loop waking for this queue.

use super::attention::{self, Act, Answer, Effect, Event, Found, Registry, State};
use crate::time::Instant;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::io::ErrorKind;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::Duration;
use wayland_backend::client::{Backend, ObjectId, WaylandError};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_display::WlDisplay;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{self, XdgActivationTokenV1};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use winit::window::{UserAttentionType, Window};

/// One native window's attention state (`attention::State`) over its binding.
pub(crate) type Attention = State<Activator>;

/// One attention request on a shown Wayland window of an app with `App::attention_wait`:
/// `token` is the one `open(token)` forwarded, else the window asks for attention. `at` is when
/// the request was made; nothing it does waits past `at + limit`, and a request handled after
/// that waits for nothing.
pub(crate) fn request(state: &mut Attention, window: &Window, token: Option<&str>, at: Instant, limit: Duration) {
    if let State::Unresponsive(a) = state && let Some(event) = a.answered().event() {
        carry(state, event);
    }
    let deadline = attention::deadline(at, limit);
    let stale = crate::time::now() >= deadline;
    if attention::binds(state, stale) {
        let event = Activator::bind(window, Some(deadline));
        carry(state, event);
    }
    let found = matches!(&*state, State::Ready(a) | State::Unresponsive(a) if a.found());
    let act = attention::act(state, token.is_some(), found, stale);
    let (State::Ready(a) | State::Unresponsive(a)) = state else { return urgent(window) };
    let wait = match (act, token) {
        (Act::ActivateSynced, Some(token)) => a.activate_synced(token, deadline),
        (Act::Attend, _) => a.attend(deadline),
        // Sending a token never blocks: a stale request, or a binding that stopped answering,
        // still hands it over without waiting for the compositor.
        (Act::Activate, Some(token)) => return a.activate(token),
        _ => return urgent(window),
    };
    carry(state, wait.event());
}

/// Without `App::attention_wait`: binds until the window has a binding, then activates without
/// waiting. Every forwarded token probes again while the global is missing.
pub(crate) fn forward(state: &mut Attention, window: &Window, token: &str) {
    if let Some(a) = attention::forward_with(state, || Activator::bind(window, None)) { a.activate(token); }
}

/// Without a token the compositor may only mark the window urgent.
pub(crate) fn urgent(window: &Window) { window.request_user_attention(Some(UserAttentionType::Informational)); }

fn carry(state: &mut Attention, event: Event<Activator>) {
    for effect in attention::step(state, event) {
        match effect {
            Effect::LogTimeout => eprintln!(
                "rune: the compositor didn't confirm a window's attention request in time; the window's later requests don't wait until it answers"
            ),
        }
    }
}

/// How a wait on the private queue ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wait {
    Met,
    TimedOut,
    Broken,
}

impl Wait {
    fn event(self) -> Event<Activator> {
        match self {
            Wait::Met => Event::Met,
            Wait::TimedOut => Event::TimedOut,
            Wait::Broken => Event::Broken,
        }
    }
}

/// The bind's sync; token requests and later syncs take the serials after it.
const BIND: u64 = 0;

/// What the private queue's events left for the activator.
#[derive(Default)]
struct Replies {
    registry: Registry,
    /// Serials of the bind's sync, token requests and syncs not answered yet.
    owed: Vec<u64>,
    /// Tokens the compositor handed back, by request serial; a late one is only an answer.
    tokens: Vec<(u64, String)>,
    /// Token requests still in flight: destroyed when their `done` comes, or with the activator.
    live: Vec<XdgActivationTokenV1>,
}

impl Replies {
    fn answered(&mut self, serial: u64) { self.owed.retain(|s| *s != serial); }
}

impl Dispatch<WlRegistry, ()> for Replies {
    fn event(r: &mut Self, _: &WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_registry::Event::Global { name, interface, .. } => r.registry.on_global(name, &interface),
            wl_registry::Event::GlobalRemove { name } => r.registry.on_global_remove(name),
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, u64> for Replies {
    fn event(r: &mut Self, _: &WlCallback, event: wl_callback::Event, serial: &u64, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = event {
            if *serial == BIND { r.registry.on_synced(); }
            r.answered(*serial);
        }
    }
}

impl Dispatch<XdgActivationV1, ()> for Replies {
    fn event(_: &mut Self, _: &XdgActivationV1, _: <XdgActivationV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<XdgActivationTokenV1, u64> for Replies {
    fn event(r: &mut Self, token: &XdgActivationTokenV1, event: xdg_activation_token_v1::Event, serial: &u64, _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_activation_token_v1::Event::Done { token: handed } = event {
            token.destroy();
            r.live.retain(|t| t != token);
            r.tokens.push((*serial, handed));
            r.answered(*serial);
        }
    }
}

/// What an activator has sent, for the native tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Token requests (`attend`).
    pub(crate) exchanges: u32,
    pub(crate) activates: u32,
    pub(crate) syncs: u32,
}

/// One window's activation binding, made on first use and kept with the native window.
pub(crate) struct Activator {
    queue: EventQueue<Replies>,
    replies: Replies,
    display: WlDisplay,
    registry: WlRegistry,
    /// Bound once the registry's list is known to hold the global.
    activation: Option<XdgActivationV1>,
    surface: WlSurface,
    serial: u64,
    counts: Counts,
}

impl Activator {
    /// Lists the globals and binds `xdg_activation_v1`, waiting for the list until `deadline`
    /// (with none, until it comes). A bind that runs out keeps its half-made binding, which a
    /// late answer finishes (`answered`). A global withdrawn before the bind isn't bound.
    pub(crate) fn bind(window: &Window, deadline: Option<Instant>) -> Event<Activator> {
        let Some((conn, surface)) = handles(window) else { return Event::BindMissing };
        Activator::start(&conn, surface).listed(deadline)
    }

    /// Asks for the global list on a private queue of `conn`: `get_registry`, then a sync.
    fn start(conn: &Connection, surface: WlSurface) -> Activator {
        let queue = conn.new_event_queue();
        let qh = queue.handle();
        let display = conn.display();
        let registry = display.get_registry(&qh, ());
        display.sync(&qh, BIND);
        let replies = Replies { owed: vec![BIND], ..Replies::default() };
        Activator { queue, replies, display, registry, activation: None, surface, serial: BIND, counts: Counts::default() }
    }

    /// Waits for the list `start` asked for until `deadline`, then binds the global if it's there.
    fn listed(mut self, deadline: Option<Instant>) -> Event<Activator> {
        match self.dispatch_until(|r| r.registry.outcome().is_some(), deadline) {
            Wait::Met if self.finish_bind() => Event::Bound(self),
            Wait::Met => Event::BindMissing,
            Wait::TimedOut => Event::BindTimedOut(self),
            Wait::Broken => Event::Broken,
        }
    }

    fn finish_bind(&mut self) -> bool {
        if self.activation.is_none() && let Some(Found::Global(name)) = self.replies.registry.outcome() {
            self.activation = Some(self.registry.bind(name, 1, &self.queue.handle(), ()));
        }
        self.activation.is_some()
    }

    /// Whether the global is bound.
    pub(crate) fn found(&self) -> bool { self.activation.is_some() }

    #[allow(dead_code, reason = "the native attention tests read it")]
    pub(crate) fn counts(&self) -> Counts { self.counts }

    /// Token requests whose `done` hasn't come.
    #[allow(dead_code, reason = "the native attention tests read it")]
    pub(crate) fn live_tokens(&self) -> usize { self.replies.live.len() }

    /// Whether every reply this binding was owed has come, without waiting: winit's loop reads
    /// the socket, which queues them here.
    pub(crate) fn answered(&mut self) -> Answer {
        if self.queue.dispatch_pending(&mut self.replies).is_err() { return Answer::Broken; }
        if !self.replies.owed.is_empty() { return Answer::Pending; }
        match self.replies.registry.outcome() {
            Some(Found::Missing) => Answer::Missing,
            Some(Found::Global(_)) if self.finish_bind() => Answer::Ready,
            _ => Answer::Pending,
        }
    }

    /// Asks for attention: gets a token for the surface, activates with it, and waits until the
    /// compositor has processed the activate (a later sync came back).
    pub(crate) fn attend(&mut self, deadline: Instant) -> Wait {
        self.replies.tokens.clear();
        let serial = self.next_serial();
        let Some(activation) = &self.activation else { return Wait::Broken };
        let token = activation.get_activation_token(&self.queue.handle(), serial);
        token.set_surface(&self.surface);
        token.commit();
        self.replies.live.push(token);
        self.replies.owed.push(serial);
        self.counts.exchanges += 1;
        let wait = self.dispatch_until(|r| !r.owed.contains(&serial), Some(deadline));
        if wait != Wait::Met { return wait; }
        let Some(at) = self.replies.tokens.iter().position(|(s, _)| *s == serial) else { return Wait::Broken };
        let (_, handed) = self.replies.tokens.swap_remove(at);
        self.activate_synced(&handed, deadline)
    }

    /// Activates with `token` and waits until the compositor has processed it.
    pub(crate) fn activate_synced(&mut self, token: &str, deadline: Instant) -> Wait {
        self.send_activate(token);
        let serial = self.next_serial();
        self.display.sync(&self.queue.handle(), serial);
        self.replies.owed.push(serial);
        self.counts.syncs += 1;
        self.dispatch_until(|r| !r.owed.contains(&serial), Some(deadline))
    }

    /// Asks the compositor to focus the window, without waiting. It decides: an expired or
    /// foreign token is ignored, or only marks the window urgent.
    pub(crate) fn activate(&mut self, token: &str) {
        self.send_activate(token);
        let _ = self.queue.flush();
    }

    fn send_activate(&mut self, token: &str) {
        if let Some(activation) = &self.activation {
            activation.activate(token.to_owned(), &self.surface);
            self.counts.activates += 1;
        }
    }

    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    /// Dispatches this queue until `done`, reading the socket while it waits, until `deadline`.
    fn dispatch_until(&mut self, done: impl Fn(&Replies) -> bool, deadline: Option<Instant>) -> Wait {
        loop {
            if self.queue.dispatch_pending(&mut self.replies).is_err() { return Wait::Broken; }
            if done(&self.replies) { return Wait::Met; }
            let want_write = match self.queue.flush() {
                Ok(()) => false,
                Err(WaylandError::Io(e)) if e.kind() == ErrorKind::WouldBlock => true,
                Err(_) => return Wait::Broken,
            };
            // Events already queued for this queue: dispatch them first.
            let Some(guard) = self.queue.prepare_read() else { continue };
            match poll_fd(guard.connection_fd(), want_write, deadline) {
                Poll::Readable => match guard.read() {
                    Ok(_) => {}
                    // Another reader took what was there.
                    Err(WaylandError::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(_) => return Wait::Broken,
                },
                // Room to send again: the next round flushes.
                Poll::Writable => drop(guard),
                Poll::TimedOut => return Wait::TimedOut,
                Poll::Hangup | Poll::Failed => return Wait::Broken,
            }
        }
    }
}

impl Drop for Activator {
    fn drop(&mut self) {
        for token in self.replies.live.drain(..) { token.destroy(); }
        if let Some(activation) = &self.activation { activation.destroy(); }
        let _ = self.queue.flush();
    }
}

/// winit's display connection and the window's surface; None off Wayland.
#[allow(unsafe_code)]
fn handles(window: &Window) -> Option<(Connection, WlSurface)> {
    let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(handle)) =
        (window.display_handle().ok()?.as_raw(), window.window_handle().ok()?.as_raw())
    else {
        return None;
    };
    // SAFETY: winit owns this `wl_display` and keeps it alive for the event loop, which outlives
    // every window; a foreign backend never closes it.
    let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    // SAFETY: the pointer is the live `wl_surface` proxy winit created for this window, and the
    // activator is dropped before the window.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), handle.surface.as_ptr().cast()) }.ok()?;
    let surface = WlSurface::from_id(&conn, id).ok()?;
    Some((conn, surface))
}

/// What waiting on the connection's socket found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Poll {
    Readable,
    /// Room to send, while a flush had to stop.
    Writable,
    /// The other end hung up with nothing left to read, or the descriptor is invalid.
    Hangup,
    TimedOut,
    Failed,
}

/// Waits until `fd` is readable (or writable, with `want_write`) or `deadline` passes. The
/// timeout rounds up to whole milliseconds, so the wait never ends early or spins; a signal
/// resumes it with the time left.
#[allow(unsafe_code)]
pub(crate) fn poll_fd(fd: BorrowedFd, want_write: bool, deadline: Option<Instant>) -> Poll {
    let events = libc::POLLIN | if want_write { libc::POLLOUT } else { 0 };
    loop {
        let timeout = match deadline {
            None => -1,
            Some(deadline) => {
                let left = deadline.saturating_duration_since(crate::time::now());
                if left.is_zero() { return Poll::TimedOut; }
                left.as_micros().div_ceil(1000).clamp(1, i32::MAX as u128) as libc::c_int
            }
        };
        let mut pfd = libc::pollfd { fd: fd.as_raw_fd(), events, revents: 0 };
        // SAFETY: one valid `pollfd` for the duration of the call.
        let n = unsafe { libc::poll(&mut pfd, 1, timeout) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == ErrorKind::Interrupted { continue; }
            return Poll::Failed;
        }
        // None ready: the loop checks the clock again.
        if n == 0 { continue; }
        let r = pfd.revents;
        if r & libc::POLLIN != 0 { return Poll::Readable; }
        if r & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 { return Poll::Hangup; }
        if r & libc::POLLOUT != 0 { return Poll::Writable; }
    }
}

#[cfg(test)]
test_group! { core =>
#[cfg(test)]
#[path = "activation_tests.rs"]
mod tests;
}
