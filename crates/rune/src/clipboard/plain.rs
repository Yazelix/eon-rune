//! Explicit plain-text operations on the OS clipboard, even in headless runs.
//! Completions run off the UI thread, except immediate unsupported/drop results;
//! marshal UI changes through AppHandle, return promptly, and never wait on this worker.
use super::*;
use std::io::Read;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Regular,
    Primary,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Text(String),
    Empty,
    NonText,
    TooLarge,
    InvalidUtf8,
    Unsupported,
    HostFailure,
    Written,
}
pub(super) struct Completion(Option<Box<dyn FnOnce(Outcome) + Send>>);
impl Completion {
    fn finish(&mut self, outcome: Outcome) {
        if let Some(f) = self.0.take() {
            drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(outcome))));
        }
    }
}
impl Drop for Completion {
    fn drop(&mut self) { self.finish(Outcome::HostFailure); }
}
pub(super) enum Operation {
    Read(bool, usize, Completion),
    Write(bool, String, Completion),
}
impl Operation {
    pub(super) fn run<B: Backend>(self, core: &mut Core, cb: &mut B) {
        match self {
            Self::Read(primary, cap, mut done) => done.finish(cb.bounded_text(primary, cap)),
            Self::Write(primary, text, mut done) => {
                let result =
                    if primary { cb.write_primary(text) } else { core.try_write(cb, ClipboardWrite::new(ClipboardContent::new(text)), None, &RealTime) };
                done.finish(if result.is_ok() { Outcome::Written } else { Outcome::HostFailure });
            }
        }
    }
}
fn submit(selection: Selection, complete: impl FnOnce(Outcome) + Send + 'static, operation: impl FnOnce(bool, Completion) -> Operation) {
    let mut done = Completion(Some(Box::new(complete)));
    let primary = selection == Selection::Primary;
    if primary && !cfg!(target_os = "linux") {
        return done.finish(Outcome::Unsupported);
    }
    drop(native::worker().send(Request::Plain(operation(primary, done))));
}
/// Reads only text. Wayland consumes at most cap+1 bytes; Vec capacity may grow further.
/// macOS checks after arboard allocates the string. Other bounded-read backends are unsupported.
pub fn read_text(selection: Selection, cap: usize, complete: impl FnOnce(Outcome) + Send + 'static) {
    submit(selection, complete, |primary, done| Operation::Read(primary, cap, done));
}
/// Reports native acceptance, not persistence. Direct operations follow queue order;
/// Tx copies enqueue later and have no same-handler ordering guarantee with these calls.
pub fn write_text(selection: Selection, text: String, complete: impl FnOnce(Outcome) + Send + 'static) {
    submit(selection, complete, |primary, done| Operation::Write(primary, text, done));
}
pub(super) fn decode(input: impl Read, cap: usize) -> Outcome {
    let mut bytes = Vec::new();
    if input.take(cap.saturating_add(1) as u64).read_to_end(&mut bytes).is_err() {
        return Outcome::HostFailure;
    }
    if bytes.len() > cap {
        return Outcome::TooLarge;
    }
    if bytes.is_empty() {
        return Outcome::Empty;
    }
    String::from_utf8(bytes).map_or(Outcome::InvalidUtf8, Outcome::Text)
}
#[cfg(any(target_os = "linux", test))]
pub(super) fn text_mime(types: &[String]) -> Option<&'static str> {
    ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"].into_iter().find(|mime| types.iter().any(|t| t == mime))
}
#[cfg(any(target_os = "macos", test))]
pub(super) fn missing_text(items: usize) -> Outcome { if items == 0 { Outcome::Empty } else { Outcome::NonText } }

#[cfg(test)]
test_group! { native =>
#[path = "plain_tests.rs"]
mod tests;
}
