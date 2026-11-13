use super::*;
use crate::{Key, Mods, PointerButton, TextEdit, TextStyle, col, el, text, text_input};

#[derive(Default)]
struct Fields {
    first: TextEdit,
    second: TextEdit,
    secure: bool,
    middle: usize,
    override_middle: bool,
    pasted: String,
    copies: std::rc::Rc<std::cell::Cell<usize>>,
}
fn view(s: &Fields, _: &mut Cx<Fields>) -> El {
    let copies = s.copies.clone();
    let font = TextStyle::new(16.0, 400.0);
    let mut first = text_input(&s.first, font)
        .key("first")
        .size(300.0, 40.0)
        .on_edit(|s: &mut Fields| &mut s.first)
        .on_input_key(|s: &mut Fields, ev, _| {
            if ev.key == Key::Function(9) {
                s.first.text.replace_range(0..4, "edit");
                s.first.set_selection(0, 4);
                return true;
            }
            if ev.key != Key::Function(7) {
                return false;
            }
            s.first.set_selection(5, 9);
            true
        });
    if s.secure {
        first = first.secure();
    }
    let mut root = col()
        .size(300.0, 160.0)
        .on_paste(|s: &mut Fields, text, _| s.pasted = text.into())
        .child(first)
        .child(
            text_input(&s.second, font)
                .key("second")
                .size(300.0, 40.0)
                .on_edit(|s: &mut Fields| &mut s.second),
        )
        .child(
            text("read only café 日本語")
                .key("label")
                .selectable(true)
                .selection_copy(move |selection| {
                    copies.set(copies.get() + 1);
                    Some(selection.text().to_owned())
                })
                .size(300.0, 40.0),
        );
    if s.override_middle {
        root = root
            .pointer_buttons([PointerButton::Middle])
            .on_press(|s: &mut Fields| s.middle += 1);
    }
    root.child(el())
}
fn runtime(secure: bool) -> (Runtime<Fields>, crate::clipboard::MemoryClipboard) {
    let mut s = Fields {
        secure,
        ..Default::default()
    };
    s.first = TextEdit::new("Rune café 日本語");
    let mut rt = Runtime::new(s, view, (300.0, 160.0), Env::new(true, 1.0));
    let board = crate::clipboard::memory();
    board.set_text("regular sentinel");
    board.set_primary_text("external sentinel");
    rt.use_clipboard(&board);
    rt.frame(0.0);
    rt.pointer_down(5.0, 20.0);
    rt.pointer_up(5.0, 20.0);
    rt.frame(0.0);
    (rt, board)
}
fn select(rt: &mut Runtime<Fields>) {
    let mods = Mods {
        ctrl: true,
        cmd: true,
        ..Default::default()
    };
    rt.physical_with_fallback(
        crate::PhysicalInput::Key {
            key: crate::PhysicalKeyCode::KeyA,
            mods,
            pressed: true,
            repeat: false,
        },
        Some(KeyEvent {
            key: Key::Command('a'),
            mods,
        }),
    );
    rt.frame(0.0);
}

#[test]
fn primary_selection_exports_without_changing_regular_or_reclaiming_another_client() {
    let (mut rt, board) = runtime(false);
    select(&mut rt);
    assert_eq!(board.primary_text().as_deref(), Some("Rune café 日本語"));
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("regular sentinel")
    );
    board.set_primary_text("another client");
    rt.window_focus(true);
    rt.frame(1.0);
    rt.window_focus(false);
    rt.frame(1.0);
    rt.window_focus(true);
    rt.frame(1.0);
    rt.frame(1.0);
    rt.frame(2.0);
    rt.key(KeyEvent {
        key: Key::Command('c'),
        mods: Mods {
            ctrl: true,
            cmd: true,
            ..Default::default()
        },
    });
    rt.frame(2.0);
    assert_eq!(board.primary_text().as_deref(), Some("another client"));
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("Rune café 日本語")
    );
}

#[test]
fn primary_selection_never_exports_secure_fields() {
    let (mut rt, board) = runtime(true);
    select(&mut rt);
    assert_eq!(board.primary_text().as_deref(), Some("external sentinel"));
}

#[test]
fn primary_selection_middle_paste_places_caret_in_clicked_field_and_keeps_regular() {
    let (mut rt, board) = runtime(false);
    board.set_primary_text(" paste 日本語");
    rt.button_down(290.0, 20.0, PointerButton::Middle);
    rt.button_up(290.0, 20.0, PointerButton::Middle);
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(rt.state.first.text, "Rune café 日本語 paste 日本語");
    assert!(rt.state.second.text.is_empty());
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("regular sentinel")
    );
    rt.pointer_down(5.0, 20.0);
    rt.pointer_up(5.0, 20.0);
    rt.frame(0.0);
    assert_eq!(
        rt.state.first.anchor, rt.state.first.focus,
        "middle click breaks the previous left-click run"
    );
    let target = rt.paste_target();
    rt.pointer_down(5.0, 60.0);
    rt.pointer_up(5.0, 60.0);
    rt.frame(0.0);
    rt.paste_result(target, crate::ClipboardContent::new("late primary reply"));
    rt.frame(0.0);
    assert!(rt.state.second.text.is_empty());
}

#[test]
fn primary_selection_middle_handler_takes_precedence() {
    let (mut rt, _) = runtime(false);
    rt.update(|s, _| s.override_middle = true);
    rt.frame(0.0);
    rt.button_down(290.0, 20.0, PointerButton::Middle);
    rt.button_up(290.0, 20.0, PointerButton::Middle);
    rt.frame(0.0);
    assert_eq!(rt.state.middle, 1);
    assert_eq!(rt.state.first.text, "Rune café 日本語");
}

#[test]
fn primary_selection_exports_read_only_pointer_selection() {
    let (mut rt, board) = runtime(false);
    rt.pointer_down(1.0, 100.0);
    rt.frame(0.0);
    rt.pointer_move(280.0, 100.0);
    rt.frame(0.0);
    assert_eq!(
        board.primary_text().as_deref(),
        Some("external sentinel"),
        "drag writes coalesce until release"
    );
    rt.pointer_up(280.0, 100.0);
    rt.frame(0.0);
    assert_eq!(
        board.primary_text().as_deref(),
        Some("read only café 日本語")
    );
}

#[test]
fn primary_selection_absent_owner_does_not_fall_back_to_regular_clipboard() {
    let (mut rt, _) = runtime(false);
    let board = crate::clipboard::memory();
    board.set_text("must not paste this");
    rt.use_clipboard(&board);
    rt.button_down(290.0, 20.0, PointerButton::Middle);
    rt.button_up(290.0, 20.0, PointerButton::Middle);
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(rt.state.first.text, "Rune café 日本語");
}

#[test]
fn primary_selection_new_range_with_identical_bytes_reclaims_ownership() {
    let (mut rt, board) = runtime(false);
    rt.update(|s, _| {
        s.first = TextEdit::new("same same");
        s.first.focused = true;
        s.first.set_selection(0, 4);
    });
    rt.frame(0.0);
    board.set_primary_text("other owner");
    rt.key(KeyEvent {
        key: Key::Function(7),
        mods: Default::default(),
    });
    rt.frame(0.0);
    assert_eq!(board.primary_text().as_deref(), Some("same"));
}

#[test]
fn primary_selection_host_can_blur_a_field_and_request_app_paste_together() {
    let (mut rt, board) = runtime(false);
    board.set_primary_text("terminal input");
    rt.update(|_, tx| {
        tx.blur();
        tx.request_primary_paste();
    });
    rt.frame(0.0);
    assert_eq!(rt.state.first.text, "Rune café 日本語");
    assert_eq!(rt.state.pasted, "terminal input");
}

#[test]
fn primary_selection_menu_select_all_exports_and_copy_does_not_reclaim() {
    for secure in [false, true] {
        let (mut rt, board) = runtime(secure);
        rt.button_down(5.0, 20.0, PointerButton::Secondary);
        rt.button_up(5.0, 20.0, PointerButton::Secondary);
        rt.frame(0.0);
        rt.resolve_popup(Some("select-all")).unwrap();
        rt.frame(0.0);
        assert_eq!(
            board.primary_text().as_deref(),
            Some(if secure {
                "external sentinel"
            } else {
                "Rune café 日本語"
            })
        );
        assert_eq!(
            board.text().as_deref().map(String::as_str),
            Some("regular sentinel")
        );
        if !secure {
            board.set_primary_text("another client");
            rt.button_down(5.0, 20.0, PointerButton::Secondary);
            rt.button_up(5.0, 20.0, PointerButton::Secondary);
            rt.frame(0.0);
            rt.resolve_popup(Some("copy")).unwrap();
            rt.frame(0.0);
            assert_eq!(board.primary_text().as_deref(), Some("another client"));
            rt.update(|_, tx| {
                tx.popup_menu(
                    crate::PopupMenu::new().item(crate::PopupItem::new("select", "Select")),
                    "first",
                    |s: &mut Fields, _, _| s.first.set_selection(0, 4),
                );
            });
            rt.frame(0.0);
            rt.resolve_popup(Some("select")).unwrap();
            rt.frame(0.0);
            assert_eq!(board.primary_text().as_deref(), Some("Rune"));
        }
    }
}

#[test]
fn primary_selection_exports_changed_text_at_the_same_range() {
    let (mut rt, board) = runtime(false);
    rt.update(|s, _| {
        s.first = TextEdit::new("same more");
        s.first.focused = true;
        s.first.set_selection(0, 4);
    });
    rt.frame(0.0);
    board.set_primary_text("another client");
    rt.key(KeyEvent {
        key: Key::Function(9),
        mods: Default::default(),
    });
    rt.frame(0.0);
    assert_eq!(board.primary_text().as_deref(), Some("edit"));
}

#[test]
fn primary_selection_copies_once_per_real_read_only_change() {
    let (mut rt, board) = runtime(false);
    for expected in [1, 2] {
        // Separate drags so the second press is not a word-selecting double click.
        rt.frame(1.0);
        board.set_primary_text("another client");
        rt.pointer_down(1.0, 100.0);
        rt.frame(0.0);
        rt.pointer_move(280.0, 100.0);
        rt.frame(0.0);
        rt.pointer_up(280.0, 100.0);
        rt.frame(0.0);
        assert_eq!(
            board.primary_text().as_deref(),
            Some("read only café 日本語")
        );
        assert_eq!(rt.state.copies.get(), expected);
        rt.key(KeyEvent {
            key: Key::Function(8),
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert_eq!(
            rt.state.copies.get(),
            expected,
            "unchanged action recopied the selection"
        );
    }
}
