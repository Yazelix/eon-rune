use super::*;
use crate::{Key, Mods, PointerButton, TextEdit, TextStyle, col, el, text, text_input};

#[derive(Clone, Copy, Default)]
enum ReleaseOverride {
    #[default]
    None,
    Range,
    Collapse,
    Explicit,
}

#[derive(Default)]
struct Fields {
    first: TextEdit,
    second: TextEdit,
    secure: bool,
    middle: usize,
    override_middle: bool,
    pasted: String,
    copies: std::rc::Rc<std::cell::Cell<usize>>,
    release: ReleaseOverride,
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
                .on_selection_change(|s: &mut Fields, _, tx| match s.release {
                    ReleaseOverride::None => {},
                    ReleaseOverride::Range => tx.select_text("label", 5..9),
                    ReleaseOverride::Collapse => tx.select_text("label", 0..0),
                    ReleaseOverride::Explicit => {
                        tx.select_text("label", 5..9);
                        tx.copy_primary("release override");
                    }
                })
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

#[test]
fn primary_release_exports_after_selection_handlers_and_programmatic_selection() {
    for (mode, expected) in [
        (ReleaseOverride::Range, "only"),
        (ReleaseOverride::Collapse, "external sentinel"),
        (ReleaseOverride::Explicit, "release override"),
    ] {
        let (mut rt, board) = runtime(false);
        rt.pointer_down(1.0, 100.0);
        rt.pointer_move(280.0, 100.0);
        rt.state.release = mode;
        rt.pointer_up(280.0, 100.0);
        crate::clipboard::flush_memory(&mut rt);
        assert_eq!(board.primary_text().as_deref(), Some("external sentinel"), "release must await handlers");
        rt.frame(0.0);
        assert_eq!(board.primary_text().as_deref(), Some(expected));
        assert_eq!(board.text().as_deref().map(String::as_str), Some("regular sentinel"));
    }
}

#[test]
fn primary_middle_paste_does_not_enter_secure_fields() {
    let (mut rt, board) = runtime(true);
    board.set_primary_text("must not paste");
    rt.button_down(290.0, 20.0, PointerButton::Middle);
    rt.button_up(290.0, 20.0, PointerButton::Middle);
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(rt.state.first.text, "Rune café 日本語");
    assert!(rt.state.pasted.is_empty());
}

#[test]
fn primary_release_validates_selection_changed_in_a_held_frame() {
    for (mode, expected) in [
        (ReleaseOverride::Range, "only"),
        (ReleaseOverride::Collapse, "external sentinel"),
        (ReleaseOverride::Explicit, "release override"),
    ] {
        let (mut rt, board) = runtime(false);
        rt.pointer_down(1.0, 100.0);
        rt.pointer_move(280.0, 100.0);
        rt.state.release = mode;
        rt.frame(0.0);
        assert_eq!(board.primary_text().as_deref(), Some("external sentinel"));
        rt.pointer_up(280.0, 100.0);
        rt.frame(0.0);
        assert_eq!(board.primary_text().as_deref(), Some(expected));
        assert_eq!(board.text().as_deref().map(String::as_str), Some("regular sentinel"));
    }
}

#[test]
fn primary_cancel_waits_for_the_final_collapsed_selection() {
    for focus_loss in [false, true] {
        let (mut rt, board) = runtime(false);
        rt.window_focus(true);
        rt.pointer_down(1.0, 100.0);
        rt.pointer_move(280.0, 100.0);
        rt.state.release = ReleaseOverride::Collapse;
        if focus_loss {
            rt.window_focus(false);
        } else {
            rt.key(KeyEvent { key: Key::Escape, mods: Mods::default() });
        }
        assert!(!rt.press.is_held());
        rt.frame(0.0);
        assert_eq!(board.primary_text().as_deref(), Some("external sentinel"));
        assert_eq!(board.text().as_deref().map(String::as_str), Some("regular sentinel"));
    }
}

#[test]
fn primary_explicit_export_from_held_drag_survives_without_a_rune_selection() {
    let mut rt = Runtime::new((), |_, _: &mut Cx<()>| {
        el().size(100.0, 40.0).on_drag(|_: &mut (), drag, tx| {
            if drag.phase == Phase::Down { tx.copy_primary("terminal word"); }
        })
    }, (100.0, 40.0), Env::new(true, 1.0));
    let board = crate::clipboard::memory();
    board.set_primary_text("external sentinel");
    rt.use_clipboard(&board);
    rt.frame(0.0);
    rt.pointer_down(10.0, 10.0);
    rt.frame(0.0);
    assert!(rt.primary_selection().is_none());
    assert_eq!(board.primary_text().as_deref(), Some("external sentinel"));
    rt.pointer_up(10.0, 10.0);
    rt.frame(0.0);
    assert_eq!(board.primary_text().as_deref(), Some("terminal word"));
}

#[test]
fn primary_middle_paste_skips_a_secondary_only_child() {
    let mut rt = Runtime::new(TextEdit::new(""), |edit, _: &mut Cx<TextEdit>| {
        text_input(edit, TextStyle::new(16.0, 400.0))
            .key("field").size(300.0, 40.0).on_edit(|e: &mut TextEdit| e)
            .child(el().key("badge").size(40.0, 40.0).absolute(0.0, 0.0)
                .on_secondary_press(|_: &mut TextEdit, _, _| {}))
    }, (300.0, 40.0), Env::new(true, 1.0));
    let board = crate::clipboard::memory();
    board.set_primary_text("paste");
    board.set_text("regular sentinel");
    rt.use_clipboard(&board);
    rt.frame(0.0);
    let (x, y) = rt.find("badge").expect("badge mounted");
    let field = rt.nodes.iter().position(|n| n.key.as_deref() == Some("field")).unwrap();
    let badge = rt.nodes.iter().position(|n| n.key.as_deref() == Some("badge")).unwrap();
    let raw = rt.hits.iter().rev().find(|&&(id, r)| rt.hit_contains(id, r, x, y))
        .and_then(|(id, _)| rt.node_index.get(id).copied());
    let target = rt.primary_target(x, y);
    eprintln!("PRIMARY_PASTE_ADMISSION {}", serde_json::json!({
        "field_rect": rt.drawn[field], "badge_rect": rt.drawn[badge], "point": [x, y],
        "raw_hit": raw.map(|i| &rt.nodes[i].path), "primary_target": target.map(|i| &rt.nodes[i].path),
        "field_path": rt.nodes[field].path, "badge_path": rt.nodes[badge].path,
    }));
    assert_eq!(raw, Some(badge), "the secondary-only badge owns the raw hit");
    assert_eq!(target, Some(field), "primary targeting skips the badge");
    assert!(rt.primary_target(x, y).and_then(|i| rt.textbox_input(i)).is_some());
    rt.button_down(x, y, PointerButton::Middle);
    rt.button_up(x, y, PointerButton::Middle);
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(rt.state.text, "paste");
    assert_eq!(board.text().as_deref().map(String::as_str), Some("regular sentinel"));
}

#[test]
fn primary_middle_paste_has_its_own_step_before_a_later_shift_press() {
    let (mut rt, _) = runtime(false);
    rt.button_down(290.0, 20.0, PointerButton::Middle);
    rt.button_up(290.0, 20.0, PointerButton::Middle);
    rt.held_mods.shift = true;
    rt.pointer_down(5.0, 20.0);
    assert_eq!(rt.queue.iter().filter(|a| matches!(a, Action::PrimaryPaste(..) | Action::Focus(..))).count(), 2, "paste and left focus are queued together");
    rt.frame(0.0);
    assert!(rt.state.first.anchor > rt.state.first.focus, "the later shift press extends from the middle-click caret");
}
