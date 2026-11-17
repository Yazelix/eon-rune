//! Linux's default selection export and middle-button field paste.
use super::*;
use crate::clipboard::primary::{Export, SelectionKey as PrimarySelection};
use selectable::virtual_selection::CopyRows;

/// Automatic exports wait for a frame to settle after the held press ends.
#[derive(Default)]
pub(in crate::runtime) enum Boundary {
    #[default]
    Pending,
    Ready,
}

enum BoundaryEvent {
    Queued,
    Frame { held: bool },
}

impl Boundary {
    fn step(&mut self, event: BoundaryEvent) -> crate::machine::Effects<()> {
        *self = match event {
            BoundaryEvent::Queued | BoundaryEvent::Frame { held: true } => Self::Pending,
            BoundaryEvent::Frame { held: false } => Self::Ready,
        };
        crate::machine::Effects::none()
    }
}

impl<S: 'static> Runtime<S> {
    pub(in crate::runtime) fn primary_selection(&mut self) -> Option<PrimarySelection> {
        if let Some(input) = self.focused_input() {
            if input.edit.is_secure() {
                return None;
            }
            let access = input.access.clone();
            let snapshot = |edit: &crate::TextEdit| {
                if edit.is_secure() || edit.anchor == edit.focus {
                    return None;
                }
                // TextEdit exposes mutable text; hash borrowed bytes to detect same-range edits.
                Some((
                    edit.anchor,
                    edit.focus,
                    crate::memo::hash_of(&edit.selected()),
                ))
            };
            let (anchor, focus, version) = match access {
                Some(access) => snapshot(access(states!(self)))?,
                None => snapshot(&input.edit)?,
            };
            return Some(PrimarySelection {
                path: self.focus.owned(),
                anchor,
                focus,
                far: None,
                version,
            });
        }
        self.primary_text_selection()
    }

    pub(in crate::runtime) fn primary_text_selection(&self) -> Option<PrimarySelection> {
        let sel = self.selectable.phase.selection()?;
        if sel.anchor == sel.focus && sel.far.is_none() {
            return None;
        }
        Some(PrimarySelection {
            path: Some(sel.path.clone()),
            anchor: sel.anchor,
            focus: sel.focus,
            far: sel.far.as_ref().map(|f| f.path.clone()),
            version: crate::memo::hash_of(&(
                &sel.content,
                sel.far.as_ref().map(|f| &f.content),
                sel.between,
            )),
        })
    }

    fn primary_selection_text(&mut self) -> Option<String> {
        if let Some(input) = self.focused_input() {
            if input.edit.is_secure() {
                return None;
            }
            let access = input.access.clone();
            let edit = match access {
                Some(ref access) => access(states!(self)),
                None => &input.edit,
            };
            return (!edit.is_secure() && edit.anchor != edit.focus)
                .then(|| edit.selected().to_owned());
        }
        self.copy_text_with(CopyRows::Retained)
    }

    fn primary_export(
        before: Option<PrimarySelection>,
        after: Option<PrimarySelection>,
        text: impl FnOnce() -> Option<String>,
    ) -> Option<Export> {
        if after == before {
            return None;
        }
        after.map(|selection| Export::Automatic {
            selection,
            text: text(),
        })
    }

    pub(in crate::runtime) fn export_primary_selection(
        &mut self,
        before: Option<PrimarySelection>,
        tx: &mut Tx,
    ) {
        if cfg!(target_os = "linux") && !matches!(tx.primary.copy, Some(Export::Explicit(_))) {
            let after = self.primary_selection();
            if let Some(copy) =
                Self::primary_export(before, after, || self.primary_selection_text())
            {
                tx.primary.copy = Some(copy);
            }
        }
    }

    pub(in crate::runtime) fn latch_primary(&mut self, tx: &mut Tx) {
        if let Some(copy) = tx.primary.copy.take() {
            if matches!(copy, Export::Automatic { .. }) {
                self.input_click
                    .primary_boundary
                    .step(BoundaryEvent::Queued);
            }
            self.input_click.primary.copy = Some(copy);
        }
        if tx.primary.paste.take().is_some() {
            self.input_click.primary.paste = Some(if tx.blur { None } else { self.paste_target() });
        }
    }

    pub(crate) fn take_primary(&mut self) -> crate::clipboard::primary::Requests {
        // A drag replaces one pending selection; it cannot queue a write per pointer event.
        let automatic = matches!(
            self.input_click.primary.copy,
            Some(Export::Automatic { .. })
        );
        let copy = if self.press.is_held()
            || (automatic && matches!(self.input_click.primary_boundary, Boundary::Pending))
        {
            None
        } else {
            match self.input_click.primary.copy.take() {
                Some(Export::Automatic { selection, text }) => {
                    // Compare with the payload's origin, including changes under a still pointer.
                    self.primary_selection().map(|current| {
                        let text = if current == selection {
                            text
                        } else {
                            self.primary_selection_text()
                        };
                        Export::Automatic {
                            selection: current,
                            text,
                        }
                    })
                }
                copy => copy,
            }
        };
        let paste = self.input_click.primary.paste.take();
        crate::clipboard::primary::Requests { copy, paste }
    }

    pub(in crate::runtime) fn finish_primary_frame(&mut self) {
        self.input_click
            .primary_boundary
            .step(BoundaryEvent::Frame {
                held: self.press.is_held(),
            });
    }

    pub(in crate::runtime) fn export_primary_text(&mut self, before: Option<PrimarySelection>) {
        if cfg!(target_os = "linux") {
            let after = self.primary_text_selection();
            if let Some(copy) =
                Self::primary_export(before, after, || self.copy_text_with(CopyRows::Retained))
            {
                self.input_click
                    .primary_boundary
                    .step(BoundaryEvent::Queued);
                self.input_click.primary.copy = Some(copy);
            }
        }
    }
}

#[cfg(test)]
test_group! { input =>
#[cfg(target_os = "linux")]
#[path = "primary_tests.rs"]
mod tests;
}
