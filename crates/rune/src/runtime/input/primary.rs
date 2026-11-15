//! Linux's default selection export and middle-button field paste.
use super::*;
use crate::clipboard::primary::Export;
use selectable::virtual_selection::CopyRows;

#[derive(PartialEq, Eq)]
pub(in crate::runtime) struct PrimarySelection {
    path: Option<String>,
    anchor: usize,
    focus: usize,
    far: Option<String>,
    version: u64,
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
        self.text_sel.path.as_ref()?;
        if self.text_sel.anchor == self.text_sel.focus && self.text_sel.far.is_none() {
            return None;
        }
        Some(PrimarySelection {
            path: self.text_sel.path.clone(),
            anchor: self.text_sel.anchor,
            focus: self.text_sel.focus,
            far: self.text_sel.far.as_ref().map(|f| f.0.clone()),
            version: crate::memo::hash_of(&(
                &self.text_sel.content,
                self.text_sel.far.as_ref().map(|(_, content)| content),
                self.text_sel.between,
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
        (after != before && after.is_some()).then(|| text().map_or(Export::Clear, Export::Write))
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
            self.input_click.primary.copy = Some(copy);
        }
        if tx.primary.paste.take().is_some() {
            self.input_click.primary.paste = Some(if tx.blur { None } else { self.paste_target() });
        }
    }

    pub(crate) fn take_primary(&mut self) -> crate::clipboard::primary::Requests {
        // A drag replaces one pending selection; it cannot queue a write per pointer event.
        let copy = if self.press.is_held() {
            None
        } else {
            self.input_click.primary.copy.take()
        };
        let paste = self.input_click.primary.paste.take();
        crate::clipboard::primary::Requests { copy, paste }
    }

    pub(in crate::runtime) fn export_primary_text(&mut self, before: Option<PrimarySelection>) {
        if cfg!(target_os = "linux") {
            let after = self.primary_text_selection();
            if let Some(copy) =
                Self::primary_export(before, after, || self.copy_text_with(CopyRows::Retained))
            {
                self.input_click.primary.copy = Some(copy);
            }
        }
    }
}

#[cfg(test)]
test_group! { input =>
#[cfg(all(test, target_os = "linux"))]
#[path = "primary_tests.rs"]
mod tests;
}
