//! Terminal input and scene conversion shared by the hosting spikes.
use orbit_protocol::{CellWidth, CursorShape, Screen, Underline};
use rune::{Key, KeyEvent};
use yazelix_venus::{Color, DrawCell, DrawCursor, DrawRow, DrawStyle, Scene, SceneGraphics};

const TERM_BG: Color = Color {
    r: 0x1c,
    g: 0x1c,
    b: 0x1c,
};
const TERM_FG: Color = Color {
    r: 0xbc,
    g: 0xbc,
    b: 0xbc,
};

/// Legacy xterm encoding of one Rune key. `None` leaves the key to Rune.
pub fn encode(ev: &KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    let m = ev.mods;
    let modifier = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
    let csi = |final_byte: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let alt = |mut bytes: Vec<u8>| {
        if m.alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    Some(match &ev.key {
        Key::Text(s) => alt(s.as_bytes().to_vec()),
        // Ctrl+Shift+V and Ctrl+Shift+C stay Rune's (paste and copy).
        Key::Command('v' | 'c') if m.shift => return None,
        Key::Command(c) => alt(vec![match c {
            'a'..='z' => *c as u8 - b'a' + 1,
            ' ' | '@' | '2' => 0,
            '[' | '3' => 0x1b,
            '\\' | '4' => 0x1c,
            ']' | '5' => 0x1d,
            '^' | '6' => 0x1e,
            '_' | '/' | '7' => 0x1f,
            '?' | '8' => 0x7f,
            _ => return None,
        }]),
        Key::Enter => alt(vec![b'\r']),
        Key::Backspace => alt(vec![if m.ctrl { 0x08 } else { 0x7f }]),
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![b'\t'],
        Key::Escape => vec![0x1b],
        Key::Up => csi('A'),
        Key::Down => csi('B'),
        Key::Right => csi('C'),
        Key::Left => csi('D'),
        Key::Home => csi('H'),
        Key::End => csi('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Function(n @ 1..=4) => format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes(),
        Key::Function(n @ 5..=11) => tilde([15, 17, 18, 19, 20, 21, 23][usize::from(n - 5)]),
        _ => return None,
    })
}

fn xterm(n: u8) -> Color {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let (r, g, b) = match n {
        0..=15 => BASE[usize::from(n)],
        16..=231 => {
            let n = n - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        _ => {
            let v = 8 + 10 * (n - 232);
            (v, v, v)
        }
    };
    Color { r, g, b }
}

fn color(c: vt100::Color) -> Option<Color> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(n) => Some(xterm(n)),
        vt100::Color::Rgb(r, g, b) => Some(Color { r, g, b }),
    }
}

pub fn scene_from(screen: &vt100::Screen, revision: u64, focused: bool) -> Scene {
    let (rows, cols) = screen.size();
    let (rows, cols) = (rows.get(), cols.get());
    let blank = DrawStyle {
        foreground: TERM_FG,
        background: TERM_BG,
        underline_color: TERM_FG,
        bold: false,
        italic: false,
        faint: false,
        blink: false,
        invisible: false,
        strikethrough: false,
        overline: false,
        selected: false,
        background_is_default: true,
        protected: false,
        underline: Underline::None,
    };
    let mut content = Vec::with_capacity(usize::from(rows));
    for r in 0..rows {
        let mut cells = Vec::with_capacity(usize::from(cols));
        for c in 0..cols {
            let Some(cell) = screen.cell(r, c) else {
                cells.push(DrawCell {
                    width: CellWidth::Narrow,
                    text: String::new(),
                    hyperlink: String::new(),
                    style: blank.clone(),
                });
                continue;
            };
            let (mut fg, mut bg) = (color(cell.fgcolor()), color(cell.bgcolor()));
            if cell.inverse() {
                (fg, bg) = (Some(bg.unwrap_or(TERM_BG)), Some(fg.unwrap_or(TERM_FG)));
            }
            let fg = fg.unwrap_or(TERM_FG);
            let contents = cell.contents();
            cells.push(DrawCell {
                width: if cell.is_wide() {
                    CellWidth::Wide
                } else if cell.is_wide_continuation() {
                    CellWidth::SpacerTail
                } else {
                    CellWidth::Narrow
                },
                text: if contents == " " {
                    String::new()
                } else {
                    contents.to_string()
                },
                hyperlink: String::new(),
                style: DrawStyle {
                    foreground: fg,
                    background: bg.unwrap_or(TERM_BG),
                    underline_color: fg,
                    bold: cell.bold(),
                    italic: cell.italic(),
                    faint: cell.dim(),
                    background_is_default: bg.is_none(),
                    underline: if cell.underline() {
                        Underline::Single
                    } else {
                        Underline::None
                    },
                    ..blank.clone()
                },
            });
        }
        content.push(DrawRow {
            wrapped: screen.row_wrapped(r),
            wrap_continuation: r > 0 && screen.row_wrapped(r - 1),
            cells,
        });
    }
    let (row, column) = screen.cursor_position();
    Scene {
        revision,
        columns: cols,
        rows,
        screen: if screen.alternate_screen() {
            Screen::Alternate
        } else {
            Screen::Primary
        },
        title: "live".into(),
        working_directory: String::new(),
        background: TERM_BG,
        foreground: TERM_FG,
        cursor: Some(DrawCursor {
            visible: !screen.hide_cursor(),
            blinking: false,
            password_input: false,
            shape: if focused {
                CursorShape::Block
            } else {
                CursorShape::BlockHollow
            },
            column,
            row,
            at_wide_tail: false,
            color: TERM_FG,
            explicit_color: false,
        }),
        content,
        graphics: SceneGraphics::default(),
    }
}
