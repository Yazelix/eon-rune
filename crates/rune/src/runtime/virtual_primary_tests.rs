use super::*;
use crate::clipboard::primary::Export;

fn finish(rt: &mut Runtime<Timeline>) {
    let (core, mut app) = rt.split_mut();
    core.finish_primary_frame(&mut app);
}

fn publish(rt: &mut Runtime<Timeline>) {
    let (core, app) = rt.split_mut();
    core.export_primary_text(&app, None);
}

fn automatic(rt: &mut Runtime<Timeline>) -> Option<String> {
    finish(rt);
    match rt.core_mut().take_primary().copy.expect("one primary operation") {
        Export::Automatic { text, .. } => text,
        Export::Explicit(_) => panic!("expected automatic selection export"),
    }
}

#[test]
fn keyboard_select_all_clears_once_and_stays_bounded_at_55_and_50000_rows() {
    for count in [55, 50_000] {
        let mut rt = timeline(count);
        let mounted = rt.core().commit.virtuals["#0/scroll/timeline"].range.clone();
        BUILT_ROWS.set(0);
        rt.physical_with_fallback(
            PhysicalInput::Key {
                key: crate::PhysicalKeyCode::KeyA,
                mods: Mods {
                    cmd: true,
                    ctrl: true,
                    ..Mods::default()
                },
                pressed: true,
                repeat: false,
            },
            Some(KeyEvent {
                key: Key::Command('a'),
                mods: Mods {
                    cmd: true,
                    ..Mods::default()
                },
            }),
        );
        tick(&mut rt);
        let built = BUILT_ROWS.get();
        eprintln!(
            "Select All: {count} rows, mounted {}, built {built}",
            mounted.len()
        );
        assert!(
            built <= mounted.len() + 3,
            "{count} rows: built {built}, mounted {mounted:?}"
        );
        assert_eq!(rt.core().commit.virtuals["#0/scroll/timeline"].range, mounted);
        assert_eq!(automatic(&mut rt), None);
        BUILT_ROWS.set(0);
        settle(&mut rt);
        assert_eq!(BUILT_ROWS.get(), 0, "no deferred collection");
        let (core, mut app) = rt.split_mut();
        let before = core.primary_selection(&mut app);
        let mut tx = crate::Tx::default();
        core.export_primary_selection(&mut app, before, &mut tx);
        assert_eq!(BUILT_ROWS.get(), 0, "unchanged export cannot collect rows");
        assert!(tx.primary.copy.is_none());
        assert!(
            rt.core_mut().take_primary().copy.is_none(),
            "unchanged selection must not reclaim primary"
        );
    }
}

#[test]
fn virtual_select_all_removes_previously_owned_row_text_and_preserves_full_copy() {
    let mut rt = timeline(ROWS);
    let board = crate::clipboard::memory();
    board.set_text("regular sentinel");
    rt.use_clipboard(&board);
    let g = glyphs_at(&rt, "r1/body");
    let (p0, p1) = (before(g[0]), (g[10].0 + 3.0, g[10].1));
    rt.pointer_down(p0.0, p0.1);
    rt.pointer_move(p1.0, p1.1);
    rt.pointer_up(p1.0, p1.1);
    tick(&mut rt);
    assert_eq!(board.primary_text().as_deref(), Some("Message body"));
    rt.update(|_, tx| tx.copy_primary("earlier explicit export"));
    assert_eq!(
        board.primary_text().as_deref(),
        Some("earlier explicit export")
    );
    command(&mut rt, 'a');
    assert_eq!(
        board.primary_text(),
        None,
        "obsolete row selection must not be served"
    );
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("regular sentinel")
    );
    command(&mut rt, 'c');
    let expected = (0..ROWS).map(row_expected).collect::<Vec<_>>().join("\n");
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some(expected.as_str())
    );
    assert_eq!(
        board.primary_text(),
        None,
        "explicit Copy must not reclaim primary"
    );
}

#[test]
fn drag_into_a_missing_middle_replaces_pending_write_with_clear_without_building_rows() {
    let mut rt = timeline(ROWS);
    let g = glyphs_at(&rt, "r3/head");
    let p0 = before(g[0]);
    rt.pointer_down(p0.0, p0.1);
    tick(&mut rt);
    rt.update(|_, tx| tx.copy_primary("earlier handler export"));
    assert!(matches!(
        rt.core().edit.input_click.primary.copy,
        Some(Export::Explicit(_))
    ));
    rt.pointer_move(g.last().unwrap().0 + 4.0, p0.1);
    assert!(matches!(
        rt.core().edit.input_click.primary.copy,
        Some(Export::Automatic { text: Some(_), .. })
    ));
    assert!(
        rt.core_mut().take_primary().copy.is_none(),
        "held drag coalesces writes"
    );
    for _ in 0..12 {
        rt.wheel(640.0, 210.0, -180.0);
        tick(&mut rt);
    }
    settle(&mut rt);
    let g40 = glyphs_at(&rt, "r40/head");
    let p1 = (g40.last().unwrap().0 + 4.0, g40.last().unwrap().1);
    BUILT_ROWS.set(0);
    rt.pointer_move(p1.0, p1.1);
    assert_eq!(
        BUILT_ROWS.get(),
        0,
        "drag export cannot build missing middle rows"
    );
    assert!(matches!(
        rt.core().edit.input_click.primary.copy,
        Some(Export::Automatic { text: None, .. })
    ));
    rt.pointer_up(p1.0, p1.1);
    assert_eq!(
        BUILT_ROWS.get(),
        0,
        "release cannot collect missing rows either"
    );
    finish(&mut rt);
    assert_eq!(
        BUILT_ROWS.get(),
        0,
        "after-handler release cannot build missing rows"
    );
    assert_eq!(automatic(&mut rt), None);
}

#[test]
fn pinned_single_row_can_export_without_building_any_other_row() {
    let mut rt = timeline(ROWS);
    rt.update(|_, tx| tx.select_text("r45/body", 0..12));
    tick(&mut rt);
    BUILT_ROWS.set(0);
    publish(&mut rt);
    assert_eq!(BUILT_ROWS.get(), 0);
    assert_eq!(automatic(&mut rt), Some("Message body".into()));
}

#[test]
fn missing_middle_clears_with_both_ends_pinned_or_a_header_outside_the_list() {
    for (view, group) in [
        (
            timeline_view as fn(&Timeline, &mut Cx<Timeline>) -> El,
            "timeline",
        ),
        (header_timeline_view, "scroll"),
    ] {
        let mut rt = Runtime::new(
            Timeline::new(ROWS),
            view,
            (1280.0, 420.0),
            Env::new(true, 2.0),
        );
        tick(&mut rt);
        settle(&mut rt);
        for _ in 0..6 {
            rt.wheel(640.0, 210.0, -160.0);
            tick(&mut rt);
        }
        settle(&mut rt);
        rt.update(|_, tx| tx.select_all_text(group));
        tick(&mut rt);
        let list = &rt.core().commit.virtuals["#0/scroll/timeline"];
        assert!(list.range.end < ROWS, "{group}: {:?}", list.range);
        if group == "timeline" {
            assert!(list.range.start > 0, "both endpoints must be offscreen");
            assert!(list.selection.pins.rows().iter().any(|key| key == "r0"));
            assert!(list.selection.pins.rows().iter().any(|key| key == "r54"));
        }
        BUILT_ROWS.set(0);
        publish(&mut rt);
        assert_eq!(BUILT_ROWS.get(), 0);
        assert_eq!(automatic(&mut rt), None);
    }
}

#[test]
fn newer_selection_requests_replace_earlier_explicit_exports_in_merge_and_latch() {
    let mut rt = timeline(ROWS);
    rt.update(|_, tx| tx.copy_primary("explicit"));
    command(&mut rt, 'a');
    assert_eq!(automatic(&mut rt), None);
    let mut tx = crate::Tx::default();
    tx.copy_primary("handler");
    let mut automatic = crate::Tx::default();
    automatic.primary.copy = Some(Export::Automatic {
        selection: {
            let (core, mut app) = rt.split_mut();
            core.primary_selection(&mut app)
        }.expect("selected virtual range"),
        text: None,
    });
    crate::runtime::action::merge(&mut tx, automatic);
    let (core, mut app) = rt.split_mut();
    core.latch_primary(&mut app, &mut tx);
    assert_eq!(self::automatic(&mut rt), None);
}

#[test]
fn explicit_export_wins_only_in_the_action_that_issued_it() {
    fn view(edit: &crate::TextEdit, _: &mut Cx<crate::TextEdit>) -> El {
        crate::text_input(edit, TextStyle::new(16.0, 400.0))
            .w_full()
            .h(40.0)
            .autofocus()
            .on_edit(|edit: &mut crate::TextEdit| edit)
            .on_input_key(|edit: &mut crate::TextEdit, ev, tx| {
                if ev.key != Key::Function(7) {
                    return false;
                }
                edit.set_selection(0, 4);
                tx.copy_primary("same action override");
                true
            })
    }
    let mut rt = Runtime::new(
        crate::TextEdit::new("Rune text"),
        view,
        (300.0, 80.0),
        Env::new(true, 1.0),
    );
    tick(&mut rt);
    settle(&mut rt);
    rt.pointer_down(5.0, 20.0);
    rt.pointer_up(5.0, 20.0);
    tick(&mut rt);
    assert!(
        rt.core().focused_input().is_some(),
        "the handler's field must have focus"
    );
    let mut tx = crate::Tx::default();
    tx.copy_primary("older action");
    for key in [Key::Function(7), Key::Command('a')] {
        let (core, mut app) = rt.split_mut();
        core.run_action(
            &mut app,
            crate::runtime::Action::Key {
                ev: KeyEvent {
                    key: key.clone(),
                    mods: Mods {
                        cmd: true,
                        ctrl: true,
                        ..Mods::default()
                    },
                },
                repeat: false,
                stage: false,
            },
            &mut tx,
        );
        if key == Key::Function(7) {
            assert!(
                matches!(&tx.primary.copy, Some(Export::Explicit(text)) if text == "same action override")
            );
        } else {
            assert!(
                matches!(&tx.primary.copy, Some(Export::Automatic { text: Some(text), .. }) if text == "Rune text")
            );
        }
    }
}

#[test]
fn retained_collection_rejects_a_mounted_row_missing_its_node() {
    let mut rt = timeline(ROWS);
    command(&mut rt, 'a');
    for row in 23..ROWS {
        let (core, app) = rt.split_mut();
        core.pin_offscreen_row(&app, "#0/scroll/timeline", row);
    }
    let mode = super::super::virtual_selection::CopyRows::Retained;
    assert!(
        { let (core, app) = rt.split(); core.copy_virtual_text(&app, mode) }.is_some(),
        "all selected row text is available"
    );
    let missing = rt
        .core_mut().tree.nodes
        .iter_mut()
        .find(|node| node.path == "#0/scroll/timeline/r2")
        .unwrap();
    missing.path.push_str("-missing");
    assert!(
        { let (core, app) = rt.split(); core.copy_virtual_text(&app, mode) }.is_none(),
        "never serve partial mounted data"
    );
}

#[test]
fn header_to_footer_across_two_lists_clears_when_the_second_has_missing_rows() {
    fn view(s: &Timeline, _: &mut Cx<Timeline>) -> El {
        let list = |key, count| {
            col()
                .key(format!("{key}-scroll"))
                .w_full()
                .h(150.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .key(key)
                        .w_full()
                        .growth(GrowthSpec::default())
                        .virtual_rows(
                            count,
                            ROW_H,
                            |_: &Timeline, i| format!("r{i}").into(),
                            build_timeline_row,
                        ),
                )
        };
        col().w_full().h_full().child(
            col()
                .key("group")
                .w_full()
                .grow(1.0)
                .selection_group(true)
                .children([
                    text("Header").key("header").h(32.0).selectable(true),
                    list("first", 2),
                    text("Inner header").key("inner").h(32.0).selectable(true),
                    list("second", s.count),
                    text("Footer").key("footer").h(32.0).selectable(true),
                ]),
        )
    }
    let mut rt = Runtime::new(
        Timeline::new(ROWS),
        view,
        (1280.0, 420.0),
        Env::new(true, 2.0),
    );
    tick(&mut rt);
    settle(&mut rt);
    assert_eq!(rt.core().commit.virtuals["#0/group/first-scroll/first"].range, 0..2);
    assert!(rt.core().commit.virtuals["#0/group/second-scroll/second"].range.end < ROWS);
    let board = crate::clipboard::memory();
    board.set_primary_text("previous owner");
    rt.use_clipboard(&board);
    rt.update(|_, tx| tx.select_all_text("group"));
    tick(&mut rt);
    for header in ["header", "inner"] {
        if header == "inner" {
            let first = rt
                .core_mut().tree.nodes
                .iter()
                .position(|n| n.path == "#0/group/inner")
                .unwrap();
            let last = rt
                .core_mut().tree.nodes
                .iter()
                .position(|n| n.path == "#0/group/footer")
                .unwrap();
            rt.core_mut().select_span(first, last);
        }
        board.set_primary_text("previous owner");
        assert!(
            rt.core().edit.selectable
                .phase
                .selection()
                .map(|s| s.path.as_str())
                .is_some_and(|p| p.ends_with(&format!("/{header}")))
        );
        assert!(
            rt.core().edit.selectable
                .phase
                .selection()
                .and_then(|s| s.far.as_ref())
                .is_some_and(|far| far.path.ends_with("/footer"))
        );
        BUILT_ROWS.set(0);
        publish(&mut rt);
        assert_eq!(BUILT_ROWS.get(), 0);
        assert!(
            matches!(
                rt.core().edit.input_click.primary.copy,
                Some(Export::Automatic { text: None, .. })
            ),
            "{header}: missing rows in any list must clear primary"
        );
        finish(&mut rt);
        assert_eq!(
            BUILT_ROWS.get(),
            0,
            "the final boundary cannot build missing rows"
        );
        crate::clipboard::flush_memory(rt.core_mut());
        assert_eq!(
            board.primary_text(),
            None,
            "never export the mounted subset"
        );
    }
}

#[test]
fn missing_middle_does_not_call_a_copy_provider_with_partial_text() {
    thread_local! { static COPIES: Cell<usize> = const { Cell::new(0) }; }
    fn view(s: &Timeline, cx: &mut Cx<Timeline>) -> El {
        timeline_view(s, cx).selection_copy(|selection| {
            COPIES.set(COPIES.get() + 1);
            Some(selection.text().to_owned())
        })
    }
    let mut rt = Runtime::new(
        Timeline::new(ROWS),
        view,
        (1280.0, 420.0),
        Env::new(true, 2.0),
    );
    tick(&mut rt);
    settle(&mut rt);
    COPIES.set(0);
    command(&mut rt, 'a');
    assert_eq!(automatic(&mut rt), None);
    assert_eq!(COPIES.get(), 0);
    rt.update(|_, tx| tx.select_text("r45/body", 0..12));
    tick(&mut rt);
    publish(&mut rt);
    assert_eq!(automatic(&mut rt), Some("Message body".into()));
    assert_eq!(COPIES.get(), 1);
}

#[test]
fn retained_selection_clears_for_oversized_single_text_or_a_truncated_group() {
    let cap = super::super::virtual_selection::MAX_VIRTUAL_COPY_BYTES;
    fn view(s: &Timeline, _: &mut Cx<Timeline>) -> El {
        col().w_full().h_full().child(
            col()
                .w_full()
                .h_full()
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .key("timeline")
                        .w_full()
                        .growth(GrowthSpec::default())
                        .selection_group(true)
                        .virtual_rows(
                            s.count,
                            ROW_H,
                            |_: &Timeline, i| format!("r{i}").into(),
                            |s: &Timeline, i| {
                                if i != 45 {
                                    return build_timeline_row(s, i);
                                }
                                let large = "é".repeat(
                                    super::super::virtual_selection::MAX_VIRTUAL_COPY_BYTES / 2 + 1,
                                );
                                col().h(ROW_H).children([
                                    text(large.clone()).key("first").selectable(true),
                                    text(large).key("second").selectable(true),
                                ])
                            },
                        ),
                ),
        )
    }
    let mut rt = Runtime::new(
        Timeline::new(ROWS),
        view,
        (1280.0, 420.0),
        Env::new(true, 2.0),
    );
    tick(&mut rt);
    for group in [false, true] {
        rt.update(|_, tx| {
            if group {
                tx.select_all_text("r45");
            } else {
                tx.select_text("r45/first", 0..cap + 2);
            }
        });
        tick(&mut rt);
        BUILT_ROWS.set(0);
        publish(&mut rt);
        assert_eq!(BUILT_ROWS.get(), 0);
        assert_eq!(automatic(&mut rt), None);
    }
}

#[test]
fn failed_primary_clear_is_consumed_once_and_keeps_regular_clipboard() {
    let mut rt = timeline(ROWS);
    let board = crate::clipboard::memory();
    board.set_text("regular sentinel");
    board.set_primary_text("previous primary owner");
    board.fail_writes(Some("injected clear failure"));
    rt.use_clipboard(&board);
    command(&mut rt, 'a');
    assert_eq!(
        board.primary_text().as_deref(),
        Some("previous primary owner")
    );
    assert!(
        rt.core_mut().take_primary().copy.is_none(),
        "failed clear must be consumed"
    );
    board.fail_writes(None);
    settle(&mut rt);
    assert_eq!(
        board.primary_text().as_deref(),
        Some("previous primary owner"),
        "unchanged selection must not retry a failed clear"
    );
    assert!(rt.core_mut().take_primary().copy.is_none());
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("regular sentinel")
    );
    rt.update(|_, tx| tx.copy_primary("next explicit selection"));
    assert_eq!(
        board.primary_text().as_deref(),
        Some("next explicit selection")
    );
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("regular sentinel")
    );
}
