use super::*;
use std::io::{self, Cursor};
use std::sync::mpsc::{Receiver, channel};

fn completion() -> (Completion, Receiver<Outcome>) {
    let (send, recv) = channel();
    (Completion(Some(Box::new(move |result| send.send(result).unwrap()))), recv)
}

#[test]
fn bounded_decoder_limits_consumption_before_utf8_conversion() {
    let mut input = Cursor::new(vec![b'x'; 8192]);
    assert_eq!(decode(&mut input, 5), Outcome::TooLarge);
    assert_eq!(input.position(), 6);
    assert_eq!(decode(Cursor::new("é"), 2), Outcome::Text("é".into()));
    assert_eq!(decode(Cursor::new("é"), 1), Outcome::TooLarge);
    assert_eq!(decode(Cursor::new([0xff]), 1), Outcome::InvalidUtf8);
    assert_eq!(decode(Cursor::new([]), 0), Outcome::Empty);
    assert_eq!(decode(Cursor::new([b'x']), 0), Outcome::TooLarge);
    assert_eq!(decode(Cursor::new("small"), usize::MAX), Outcome::Text("small".into()));
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> { Err(io::Error::other("injected")) }
    }
    assert_eq!(decode(Broken, 100), Outcome::HostFailure);
    assert_eq!(decode((&b"partial"[..]).chain(Broken), 100), Outcome::HostFailure, "a read that fails after a prefix drops it");
    assert_eq!(decode(io::repeat(b'x'), 1024), Outcome::TooLarge);
}

#[test]
fn native_mime_negotiation_rejects_rich_and_prefers_plain_offers() {
    let offers = |types: &[&str]| types.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    for types in [&["text/html"][..], &["text/rtf"], &["application/rtf"], &["text/html", "text/rtf"]] {
        assert_eq!(text_mime(&offers(types)), None);
    }
    assert_eq!(text_mime(&offers(&["text/html", "text/plain"])), Some("text/plain"));
    assert_eq!(text_mime(&offers(&["text/rtf", "UTF8_STRING"])), Some("UTF8_STRING"));
    assert_eq!(text_mime(&offers(&["text/plain", "text/html", "text/plain;charset=utf-8"])), Some("text/plain;charset=utf-8"));
    assert_eq!(text_mime(&offers(&["STRING"])), Some("STRING"));
}

#[test]
fn native_missing_text_distinguishes_empty_from_nontext_items() {
    assert_eq!(missing_text(0), Outcome::Empty);
    assert_eq!(missing_text(1), Outcome::NonText);
    assert_eq!(missing_text(usize::MAX), Outcome::NonText);
}

#[test]
fn abandoned_and_closed_channel_operations_complete_once() {
    let (done, recv) = completion();
    drop(Operation::Read(false, 10, done));
    assert_eq!(recv.recv().unwrap(), Outcome::HostFailure);
    assert!(recv.recv().is_err());
    let (send, requests) = channel::<Request>();
    drop(requests);
    let (done, recv) = completion();
    drop(send.send(Request::Plain(Operation::Write(false, "private".into(), done))));
    assert_eq!(recv.recv().unwrap(), Outcome::HostFailure);
    assert!(recv.recv().is_err());
}

#[derive(Default)]
struct Fake {
    regular: String,
    primary: String,
    order: Vec<String>,
}
impl Backend for Fake {
    fn write(&mut self, copy: &ClipboardWrite) -> Result<(), String> {
        self.order.push(format!("regular:{}", copy.content.text()));
        if copy.content.text() == "fail" {
            return Err("injected".into());
        }
        self.regular = copy.content.text().into();
        Ok(())
    }
    fn write_primary(&mut self, text: String) -> Result<(), String> {
        self.order.push(format!("primary:{text}"));
        if text == "fail" {
            return Err("injected".into());
        }
        self.primary = text;
        Ok(())
    }
    fn bounded_text(&mut self, primary: bool, cap: usize) -> Outcome {
        let text = if primary { &self.primary } else { &self.regular };
        decode(Cursor::new(text), cap)
    }
    fn text(&mut self) -> Option<Zeroizing<String>> { panic!("unbounded read") }
    fn content(&mut self, _: bool) -> Option<ClipboardContent> { panic!("rich read") }
    fn revision(&self) -> Option<u64> { Some(1) }
    fn clear(&mut self) -> Result<(), String> {
        self.regular.clear();
        Ok(())
    }
}

#[test]
fn operations_preserve_destination_order_and_survive_failed_writes() {
    let mut core = Core::default();
    let mut fake = Fake::default();
    let mut replies = Vec::new();
    for (primary, text) in [(false, "A"), (true, "B"), (false, "fail"), (false, "C")] {
        let (done, recv) = completion();
        Operation::Write(primary, text.into(), done).run(&mut core, &mut fake);
        replies.push(recv.recv().unwrap());
        assert!(recv.recv().is_err());
    }
    assert_eq!(replies, [Outcome::Written, Outcome::Written, Outcome::HostFailure, Outcome::Written]);
    assert_eq!(fake.order, ["regular:A", "primary:B", "regular:fail", "regular:C"]);
    for (primary, expected) in [(false, "C"), (true, "B")] {
        let (done, recv) = completion();
        Operation::Read(primary, 10, done).run(&mut core, &mut fake);
        assert_eq!(recv.recv().unwrap(), Outcome::Text(expected.into()));
        assert!(recv.recv().is_err());
    }
}

fn receive(recv: &Receiver<Outcome>) -> Outcome { recv.recv_timeout(Duration::from_secs(5)).expect("worker must complete") }

#[test]
fn worker_open_failure_reports_reads_and_writes_once() {
    let worker = native::Worker::spawn(|| None::<Fake>);
    for write in [false, true] {
        let (done, recv) = completion();
        let op = if write { Operation::Write(false, "text".into(), done) } else { Operation::Read(false, 1, done) };
        worker.sender.send(Request::Plain(op)).unwrap_or_else(|_| panic!("worker closed"));
        assert_eq!(receive(&recv), Outcome::HostFailure);
        assert!(recv.recv().is_err());
    }
    worker.stop();
}

#[test]
fn shutdown_drops_queued_operations_with_failure() {
    let (entered, opened) = channel();
    let (release, wait) = channel();
    let worker = native::Worker::spawn(move || {
        entered.send(()).unwrap();
        wait.recv().unwrap();
        Some(Fake::default())
    });
    let (first, first_reply) = completion();
    worker.sender.send(Request::Plain(Operation::Read(false, 1, first))).unwrap_or_else(|_| panic!("worker closed"));
    opened.recv_timeout(Duration::from_secs(5)).unwrap();
    worker.sender.send(Request::Shutdown).unwrap_or_else(|_| panic!("worker closed"));
    let (done, recv) = completion();
    worker.sender.send(Request::Plain(Operation::Read(false, 1, done))).unwrap_or_else(|_| panic!("worker closed"));
    release.send(()).unwrap();
    assert_eq!(receive(&first_reply), Outcome::Empty);
    assert_eq!(receive(&recv), Outcome::HostFailure);
    assert!(recv.recv().is_err());
    worker.stop();
}

#[test]
fn callback_panic_does_not_kill_shared_worker() {
    let worker = native::Worker::spawn(|| Some(Fake::default()));
    let panic = Completion(Some(Box::new(|_| panic!("injected callback panic"))));
    worker.sender.send(Request::Plain(Operation::Read(false, 1, panic))).unwrap_or_else(|_| panic!("worker closed"));
    let (done, recv) = completion();
    worker.sender.send(Request::Plain(Operation::Write(false, "after".into(), done))).unwrap_or_else(|_| panic!("worker closed"));
    assert_eq!(receive(&recv), Outcome::Written);
    worker.stop();
}

#[test]
fn plain_regular_write_replaces_secret_but_primary_does_not() {
    let mut core = Core::default();
    let mut fake = Fake::default();
    let (write, secret) = ClipboardWrite::secret("secret".into(), Duration::from_secs(3600));
    core.write(&mut fake, write, None, &RealTime);
    for primary in [true, false] {
        let (done, recv) = completion();
        Operation::Write(primary, "replacement".into(), done).run(&mut core, &mut fake);
        assert_eq!(receive(&recv), Outcome::Written);
        assert_eq!(secret.state(), if primary { SecretState::Holding } else { SecretState::Replaced });
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn unsupported_primary_finishes_without_opening_worker() {
    let (send, recv) = channel();
    submit(Selection::Primary, move |result| send.send(result).unwrap(), |_, _| panic!("must not queue"));
    assert_eq!(receive(&recv), Outcome::Unsupported);
}
