//! Linux's independent selection export and middle-button paste.

use super::*;
use crate::clipboard::primary::{Export, Requests, SelectionKey};
use selectable::virtual_selection::CopyRows;

#[derive(Default)]
pub(in crate::runtime) enum Boundary {
    #[default]
    Pending,
    Ready,
}

impl RuntimeCore {
    fn focused_primary_edit<'a>(&'a self, app: &'a mut dyn AppDriver) -> Option<(&'a InputSpec, &'a crate::TextEdit)> {
        let input = self.focused_input()?;
        if input.edit.is_secure() || self.focused_index().is_some_and(|i| {
            std::iter::successors(Some(i), |&p| self.tree.nodes[p].parent).any(|p| self.tree.nodes[p].is_secure())
        }) {
            return None;
        }
        let edit = match &input.access {
            Some(access) => access(app.states()),
            None => &input.edit,
        };
        (!edit.is_secure() && edit.anchor != edit.focus).then_some((input, edit))
    }

    pub(in crate::runtime) fn primary_selection(&self, app: &mut dyn AppDriver) -> Option<SelectionKey> {
        if self.focused_input().is_some() {
            let (_, edit) = self.focused_primary_edit(app)?;
            return Some(SelectionKey {
                path: self.kbd.focus.owned(), anchor: edit.anchor, focus: edit.focus,
                far: None, version: crate::memo::hash_of(&edit.selected()),
            });
        }
        self.primary_text_selection()
    }

    pub(in crate::runtime) fn primary_text_selection(&self) -> Option<SelectionKey> {
        let selection = self.edit.selectable.phase.selection()?;
        if selection.anchor == selection.focus && selection.far.is_none() { return None; }
        Some(SelectionKey {
            path: Some(selection.path.clone()), anchor: selection.anchor, focus: selection.focus,
            far: selection.far.as_ref().map(|far| far.path.clone()),
            version: crate::memo::hash_of(&(&selection.content, selection.far.as_ref().map(|far| &far.content), selection.between)),
        })
    }

    fn primary_selection_text(&self, app: &mut dyn AppDriver) -> Option<String> {
        if self.focused_input().is_some() {
            let (input, edit) = self.focused_primary_edit(app)?;
            return Some(match &input.pill_copy_text {
                Some(copy) => edit.selected_with_pills(|id| copy(id)),
                None => edit.selected().to_owned(),
            });
        }
        self.copy_text_with(app, CopyRows::Retained)
    }

    pub(in crate::runtime) fn export_primary_selection(&self, app: &mut dyn AppDriver, before: Option<SelectionKey>, tx: &mut Tx) {
        if !cfg!(target_os = "linux") || matches!(tx.primary.copy, Some(Export::Explicit(_))) { return; }
        let after = self.primary_selection(app);
        if after != before && let Some(selection) = after {
            tx.primary.copy = Some(Export::Automatic { selection, text: self.primary_selection_text(app) });
        }
    }

    pub(in crate::runtime) fn export_primary_text(&mut self, app: &dyn ViewDriver, before: Option<SelectionKey>) {
        if !cfg!(target_os = "linux") { return; }
        let after = self.primary_text_selection();
        if after != before && let Some(selection) = after {
            let text = self.copy_text_with(app, CopyRows::Retained);
            self.edit.input_click.primary.copy = Some(Export::Automatic { selection, text });
            self.edit.input_click.primary_boundary = Boundary::Pending;
        }
    }

    pub(in crate::runtime) fn latch_primary(&mut self, app: &mut dyn AppDriver, tx: &mut Tx) {
        if let Some(copy) = tx.primary.copy.take() {
            if matches!(copy, Export::Automatic { .. }) { self.edit.input_click.primary_boundary = Boundary::Pending; }
            self.edit.input_click.primary.copy = Some(copy);
        }
        if tx.primary.paste.take().is_some() {
            self.edit.input_click.primary.paste = Some(if tx.blur { None } else { self.paste_target() });
        }
        if matches!(self.edit.input_click.primary_boundary, Boundary::Ready) {
            self.validate_primary_export(app);
        }
    }

    /// Revalidate while the typed app driver is available. The worker only receives the
    /// payload paired with this frame's final selection.
    fn validate_primary_export(&mut self, app: &mut dyn AppDriver) {
        if !matches!(self.edit.input_click.primary.copy, Some(Export::Automatic { .. })) { return; }
        let Some(Export::Automatic { selection, text }) = self.edit.input_click.primary.copy.take() else { return };
        self.edit.input_click.primary.copy = self.primary_selection(app).map(|current| {
            let text = if current == selection { text } else { self.primary_selection_text(app) };
            Export::Automatic { selection: current, text }
        });
    }

    pub(in crate::runtime) fn finish_primary_frame(&mut self, app: &mut dyn AppDriver) {
        if self.pointer.press_held().is_some() {
            self.edit.input_click.primary_boundary = Boundary::Pending;
        } else {
            self.validate_primary_export(app);
            self.edit.input_click.primary_boundary = Boundary::Ready;
        }
    }

    pub(crate) fn take_primary(&mut self) -> Requests {
        let input = &mut self.edit.input_click;
        let waiting = matches!(input.primary.copy, Some(Export::Automatic { .. }))
            && matches!(input.primary_boundary, Boundary::Pending);
        let copy = if waiting || self.pointer.press_held().is_some() { None } else { input.primary.copy.take() };
        Requests { copy, paste: input.primary.paste.take() }
    }

    pub(in crate::runtime) fn primary_paste_allowed(&self, i: usize) -> bool {
        self.allowed(i) && !self.subtree_exiting(i) && !self.subtree_unmounted(i)
            && !self.is_node_read_only(i)
            && self.tree.nodes[i].input.as_ref().is_some_and(|input| input.access.is_some() && !input.edit.is_secure())
            && !std::iter::successors(Some(i), |&p| self.tree.nodes[p].parent)
                .any(|p| self.tree.nodes[p].is_secure() || self.tree.nodes[p].a11y.disabled)
    }

    pub(in crate::runtime) fn primary_paste_target(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let area = self.primary_target(x, y).filter(|&i| self.allowed(i) && !self.subtree_exiting(i))?;
        let input = self.textbox_input(area).filter(|&i| self.primary_paste_allowed(i))?;
        Some((area, input))
    }
}

#[cfg(test)]
test_group! { selection =>
#[cfg(test)]
#[path = "primary_selection_tests.rs"]
mod tests;
#[cfg(all(test, target_os = "linux"))]
#[path = "primary_tests.rs"]
mod behavioral_tests;
}
