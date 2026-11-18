//! Native Wayland primary-selection probe; text and terminal-host routes.
use rune::{
    AppSpec, Cx, El, Key, PointerButton, TextEdit, TextStyle, Tx, col, el, text, text_input,
};

struct State {
    edit: TextEdit,
    secure: TextEdit,
    terminal: String,
    middle: usize,
}
fn trace(element: El, key: &'static str) -> El {
    element.on_layout(move |_: &mut State, r| eprintln!("PRIMARY layout {key} {r:?}"))
}
fn init() -> State {
    State {
        edit: TextEdit::new("Rune → Wayland: café 日本語"),
        secure: TextEdit::new("never export"),
        terminal: String::new(),
        middle: 0,
    }
}
fn view(s: &State, cx: &mut Cx<State>) -> El {
    cx.web_focus();
    col()
        .w_full()
        .h_full()
        .padding(20.0)
        .gap(10.0)
        .fill(rune::color::hex("#0a0d14").unwrap())
        .on_paste(|s: &mut State, value, _| {
            s.terminal.push_str(value);
            eprintln!("PRIMARY terminal {value:?}");
        })
        .child(text("Primary selection probe"))
        .child(trace(
            text_input(&s.edit, TextStyle::new(18.0, 400.0))
                .key("field")
                .w_full()
                .h(44.0)
                .padding(6.0)
                .single_line()
                .autofocus()
                .on_edit(|s: &mut State| &mut s.edit),
            "field",
        ))
        .child(trace(
            text_input(&s.secure, TextStyle::new(18.0, 400.0))
                .key("secure")
                .w_full()
                .h(44.0)
                .padding(6.0)
                .single_line()
                .secure()
                .on_edit(|s: &mut State| &mut s.secure),
            "secure",
        ))
        .child(trace(
            text("Read-only selection café 日本語")
                .key("read-only")
                .selectable(true)
                .w_full()
                .h(44.0),
            "read-only",
        ))
        .child(trace(
            el().key("terminal")
                .focusable(true)
                .w_full()
                .h(60.0)
                .pointer_buttons([PointerButton::Primary, PointerButton::Middle])
                .on_press_tx(|s: &mut State, tx: &mut Tx| {
                    if tx.button() == PointerButton::Primary {
                        tx.focus("terminal", false);
                    } else {
                        s.middle += 1;
                        tx.blur();
                        tx.focus("terminal", false);
                        tx.request_primary_paste();
                    }
                })
                .child(text(&format!("Terminal input: {}", s.terminal))),
            "terminal",
        ))
        .child(trace(
            el().key("export-terminal")
                .w_full()
                .h(40.0)
                .on_press_tx(|_: &mut State, tx: &mut Tx| {
                    tx.copy_primary("Terminal → Wayland: 日本語")
                })
                .child(text("Export terminal selection")),
            "export-terminal",
        ))
}
fn main() {
    rune::tooling::set_app_id("org.yazelix.rune-stack-proof");
    rune::run(AppSpec {
        on_key: Some(|s: &mut State, ev, _| {
            if ev.key != Key::Function(12) {
                return false;
            }
            let value =
                serde_json::json!({"field":s.edit.text,"terminal":s.terminal,"middle":s.middle});
            eprintln!("PRIMARY probe {value}");
            true
        }),
        ..AppSpec::new("Rune primary I", (900.0, 520.0), init, view)
    });
}
