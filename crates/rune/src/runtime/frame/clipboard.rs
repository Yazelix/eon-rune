//! Clipboard requests latched by the frame.
use super::*;

impl<S: 'static> Runtime<S> {
    pub(in crate::runtime) fn latch_clipboard(&mut self, tx: &mut Tx) {
        if std::mem::take(&mut tx.clear_secret) {
            if self
                .copy_request
                .as_ref()
                .is_some_and(crate::ClipboardWrite::is_secret)
            {
                self.copy_request = None;
            }
            self.clear_secret_request = true;
            self.pill_copy = None;
        }
        if let Some(copy) = tx.copy.take() {
            self.pill_copy = (!copy.is_secret())
                .then(|| copy.content().html())
                .flatten()
                .filter(|h| h.contains("<span data-pill="))
                .map(|h| {
                    let frag = crate::TextEdit::from_html(h);
                    (
                        zeroize::Zeroizing::new(copy.content().text().to_owned()),
                        frag.pills()
                            .map(|(r, id)| (r, id.to_owned()))
                            .collect::<Vec<_>>(),
                    )
                })
                .filter(|(_, pills)| !pills.is_empty());
            self.copy_request = Some(copy);
        }
        if std::mem::take(&mut tx.paste) {
            self.paste_request = Some(self.paste_target());
        }
        for read in tx.clipboard_reads.drain(..).filter_map(|read| read.take()) {
            self.next_clipboard_read += 1;
            self.clipboard_reads.push((self.next_clipboard_read, read));
            self.read_requests.push(self.next_clipboard_read);
        }
    }
}
