use super::*;
use wayland_client::Connection;
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;

fn after(ms: u64) -> Option<Instant> { Some(Instant::now() + Duration::from_millis(ms)) }

#[test]
fn a_passed_deadline_returns_at_once() {
    let (ours, _peer) = UnixStream::pair().unwrap();
    let start = Instant::now();
    assert_eq!(poll_fd(ours.as_fd(), false, Some(start)), Poll::TimedOut);
    // Generous: a descheduled test thread mustn't fail it; a wait that ignored the deadline
    // would hang instead.
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_silent_peer_times_out_at_the_deadline_not_before() {
    let (ours, _peer) = UnixStream::pair().unwrap();
    for wait in [Duration::from_micros(400), Duration::from_millis(30)] {
        let start = Instant::now();
        assert_eq!(poll_fd(ours.as_fd(), false, Some(start + wait)), Poll::TimedOut);
        let took = start.elapsed();
        assert!(took >= wait, "ended early: {took:?} < {wait:?}");
        // Generous for a loaded host; the lower bound is the real check (no early end).
        assert!(took < wait + Duration::from_secs(1), "{took:?}");
    }
}

#[test]
fn a_write_from_the_peer_is_readable() {
    let (ours, mut peer) = UnixStream::pair().unwrap();
    peer.write_all(b"x").unwrap();
    assert_eq!(poll_fd(ours.as_fd(), false, after(1000)), Poll::Readable);
}

#[test]
fn a_closed_socket_peer_reads_as_its_end() {
    // Linux reports `POLLIN | POLLHUP`: the read that follows finds the end, which the wait
    // reports as a broken connection.
    let (ours, peer) = UnixStream::pair().unwrap();
    drop(peer);
    assert_eq!(poll_fd(ours.as_fd(), false, after(1000)), Poll::Readable);
}

#[test]
fn a_hangup_with_nothing_to_read_is_a_hangup() {
    // An empty pipe whose writer closed reports `POLLHUP` without `POLLIN` on Linux.
    let (read, write) = std::io::pipe().unwrap();
    drop(write);
    let polled = poll_fd(read.as_fd(), false, after(1000));
    if cfg!(target_os = "linux") { assert_eq!(polled, Poll::Hangup); }
    assert!(matches!(polled, Poll::Hangup | Poll::Readable), "{polled:?}");
}

#[test]
fn a_full_send_buffer_waits_until_the_peer_drains_it() {
    let (mut ours, mut peer) = UnixStream::pair().unwrap();
    ours.set_nonblocking(true).unwrap();
    let chunk = [0u8; 4096];
    while ours.write(&chunk).is_ok() {}
    assert_eq!(poll_fd(ours.as_fd(), true, after(20)), Poll::TimedOut, "nothing drained yet");
    let drain = std::thread::spawn(move || {
        let mut buf = vec![0u8; 1 << 20];
        assert!(matches!(peer.read(&mut buf), Ok(n) if n > 0));
        peer
    });
    assert_eq!(poll_fd(ours.as_fd(), true, after(2000)), Poll::Writable);
    drop(drain.join().unwrap());
}

/// A fake compositor on one end of a socket pair: it reads the client's requests and writes
/// events in the Wayland wire format, so the activator's real queue and dispatch run.
struct Server(UnixStream);

/// One request: (object, opcode, argument words).
type Request = (u32, u16, Vec<u32>);

impl Server {
    /// Every request the client has flushed so far.
    fn requests(&mut self) -> Vec<Request> {
        self.0.set_nonblocking(true).unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match self.0.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => bytes.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("{e}"),
            }
        }
        let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|w| u32::from_ne_bytes(*w)).collect();
        let mut out = Vec::new();
        let mut at = 0;
        while at < words.len() {
            let (object, head) = (words[at], words[at + 1]);
            let size = (head >> 16) as usize / 4;
            out.push((object, (head & 0xffff) as u16, words[at + 2..at + size].to_vec()));
            at += size;
        }
        out
    }

    fn send(&mut self, object: u32, opcode: u16, args: &[u32]) {
        let size = (8 + 4 * args.len()) as u32;
        let mut words = vec![object, (size << 16) | u32::from(opcode)];
        words.extend_from_slice(args);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        self.0.write_all(&bytes).unwrap();
    }
}

/// A wire string: its length with the NUL, then its bytes padded to whole words.
fn wire_string(s: &str) -> Vec<u32> {
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    let len = bytes.len() as u32;
    bytes.resize(bytes.len().div_ceil(4) * 4, 0);
    std::iter::once(len).chain(bytes.as_chunks::<4>().0.iter().map(|w| u32::from_ne_bytes(*w))).collect()
}

/// Starts an activator against a fake compositor and returns the ids of its registry and its
/// list sync, read from the client's own `get_registry` and `sync` requests.
fn started() -> (Activator, Server, u32, u32) {
    let (client, server) = UnixStream::pair().unwrap();
    let conn = Connection::from_socket(client).unwrap();
    let surface = WlSurface::inert(conn.backend().downgrade());
    let a = Activator::start(&conn, surface);
    a.queue.flush().unwrap();
    let mut server = Server(server);
    let requests = server.requests();
    // wl_display (object 1): get_registry is opcode 1, sync is opcode 0.
    let registry = requests.iter().find(|r| r.0 == 1 && r.1 == 1).expect("get_registry").2[0];
    let sync = requests.iter().find(|r| r.0 == 1 && r.1 == 0).expect("sync").2[0];
    (a, server, registry, sync)
}

/// Lists one `xdg_activation_v1` global named 46, withdrawn first if `withdrawn`, then ends the
/// list; returns the bind's result and every request the client sent after it.
fn listed(withdrawn: bool) -> (Event<Activator>, Vec<Request>) {
    let (a, mut server, registry, sync) = started();
    let mut global = vec![46];
    global.extend(wire_string(super::attention::INTERFACE));
    global.push(1);
    server.send(registry, 0, &global); // wl_registry.global
    if withdrawn { server.send(registry, 1, &[46]); } // wl_registry.global_remove
    server.send(sync, 0, &[0]); // wl_callback.done
    let mut event = a.listed(after(2000));
    if let Event::Bound(a) | Event::BindTimedOut(a) = &mut event { a.queue.flush().unwrap(); }
    (event, server.requests())
}

#[test]
fn an_advertised_global_is_bound_through_the_real_queue() {
    // The control for the next test: the harness sees a bind when there is one.
    let (event, requests) = listed(false);
    let Event::Bound(a) = &event else { panic!("expected Bound") };
    assert!(a.found());
    assert_eq!(a.counts(), Counts::default(), "a bind alone asks for no attention");
    // wl_registry.bind is opcode 0 with the global's name first.
    assert!(requests.iter().any(|r| r.1 == 0 && r.2.first() == Some(&46)), "{requests:?}");
}

#[test]
fn a_global_withdrawn_before_the_list_ends_is_never_bound() {
    let (event, requests) = listed(true);
    assert!(matches!(event, Event::BindMissing), "the withdrawn global counts as missing");
    assert!(!requests.iter().any(|r| r.2.first() == Some(&46)), "no request names the withdrawn global: {requests:?}");
}
