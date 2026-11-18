//! Stage B spike: Venus's terminal layer (rectangle pipeline and glyphon text) drawn inside Rune's
//! render hook, on Rune's device and in Rune's pass. Rune draws the bottom bar; an effect slot is
//! the terminal. `TERMGRID_CAPTURE` supplies synthetic ANSI terminal text parsed into a
//! Venus `Scene`; the fork checks generate this input without capturing a user screen.
//! `TERMGRID_MODE=static|one|all`, `TERMGRID_FONT=px`, `TERMGRID_FOCUS=1`, `TERMGRID_RESIZE=1`,
//! `TERMGRID_STATS=path` + `TERMGRID_STATS_MS`.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use orbit_protocol::{CellWidth, CursorShape, Screen, Underline};
use rune::compose::{Compositor, DeviceKey, Frame};
use rune::render::KIND_EFFECT;
use rune::tok::{self, type_};
use rune::{AppSpec, Cx, El, KeyEvent, TextEdit, Tx, col, effect, el, row, text, text_input};
use winit::dpi::PhysicalSize;
use yazelix_venus::{Color, DrawCell, DrawCursor, DrawRow, DrawStyle, FontSettings, FontSetup, Renderer, Scene, SceneGraphics};

// ---------------------------------------------------------------- Rune view code

const GRID_MARK: u32 = 1;
const COLS: usize = 200;
const ROWS: usize = 60;
const TERM_BG: Color = Color { r: 0x1c, g: 0x1c, b: 0x1c };
const TERM_FG: Color = Color { r: 0xbc, g: 0xbc, b: 0xbc };
static FRAMES: AtomicU64 = AtomicU64::new(0);
/// Nanoseconds spent in Venus's prepare, and in the whole compositor `render`, since start.
static PREP_NS: AtomicU64 = AtomicU64::new(0);
static RENDER_NS: AtomicU64 = AtomicU64::new(0);

fn env_is(name: &str, v: &str) -> bool {
    std::env::var(name).is_ok_and(|x| x == v)
}

struct State {
    field: TextEdit,
    resized: bool,
}

fn init() -> State {
    State { field: TextEdit::new(""), resized: false }
}

fn chip(label: &str, t: &tok::Theme) -> El {
    el().px(8.0).py(3.0).radius(6.0).fill(t.card).child(text(label).font(type_::LABEL).color(t.text))
}

fn view(s: &State, _: &mut Cx<State>) -> El {
    let t = tok::theme(true);
    let mut field = text_input(&s.field, type_::BODY).key("field").single_line().w(220.0).px(8.0).py(3.0)
        .radius(6.0).fill(t.control_off).color(t.text).placeholder("type here", t.text_secondary)
        .on_edit(|s: &mut State| &mut s.field);
    if env_is("TERMGRID_FOCUS", "1") {
        field = field.autofocus();
    }
    let bar = row().key("bar").w_full().h(34.0).px(8.0).gap(8.0).items_center().fill(t.bg)
        .child(chip("14:32", &t).key("clock"))
        .child(chip("eon 0.0.0", &t).key("version"))
        .child(chip("CPU 12%", &t).key("cpu"))
        .child(chip("RAM 9.1G", &t).key("ram"))
        .child(chip("disk 61%", &t).key("disk"))
        .child(el().grow(1.0))
        .child(field);
    // A red padding ring shows whether the terminal stays inside its slot; the badge is drawn by
    // Rune after the slot and must stay on top of the terminal.
    let main = col().key("main").w_full().grow(1.0).padding(8.0).fill(rune::color::hex("#aa2222").unwrap())
        .child(el().key("clipbox").w_full().grow(1.0).clip_soft(0.0).fill(rune::color::hex("#1c1c1c").unwrap())
            .child(effect(GRID_MARK).key("grid").w_full().h_full())
            .child(el().key("badge").absolute(1500.0, 24.0).px(10.0).py(6.0).radius(6.0).fill(t.bg)
                .child(text("rune badge over venus").font(type_::TITLE).color(t.text))));
    col().w_full().h_full().fill(t.bg).child(main).child(bar)
}

fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    if !env_is("TERMGRID_RESIZE", "1") || s.resized {
        return false;
    }
    if clock >= 2.0 {
        s.resized = true;
        tx.resize_window_instant((1400.0, 800.0));
        eprintln!("hosted: asked for 1400x800 at clock {clock:.2}");
        return true;
    }
    tx.wake_after(Duration::from_millis(250));
    false
}

fn on_key(s: &mut State, ev: &KeyEvent, _: &mut Tx) -> bool {
    eprintln!("hosted: on_key {ev:?} field.focused={}", s.field.focused);
    false
}

fn main() {
    let started = Instant::now();
    if let Ok(path) = std::env::var("TERMGRID_STATS") {
        let every = std::env::var("TERMGRID_STATS_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
        std::thread::spawn(move || {
            let mut out = std::fs::File::create(path).unwrap();
            loop {
                std::thread::sleep(Duration::from_millis(every));
                let _ = writeln!(out, "{:.3} {} {:.3} {:.3}", started.elapsed().as_secs_f64(), FRAMES.load(Ordering::Relaxed),
                    PREP_NS.load(Ordering::Relaxed) as f64 / 1e6, RENDER_NS.load(Ordering::Relaxed) as f64 / 1e6);
            }
        });
    }
    let mode = match std::env::var("TERMGRID_MODE").as_deref() {
        Ok("one") => Mode::One,
        Ok("all") => Mode::All,
        _ => Mode::Static,
    };
    let capture = std::env::var("TERMGRID_CAPTURE").expect("TERMGRID_CAPTURE: path to a tmux capture-pane -e -p file");
    let scene = parse_capture(&std::fs::read_to_string(capture).expect("readable capture"));
    let font = std::env::var("TERMGRID_FONT").ok().and_then(|v| v.parse().ok()).unwrap_or(14.0);
    let spec = AppSpec::<State> { poll: Some(poll), on_key: Some(on_key), ..AppSpec::new("hosted", (1920.0, 1080.0), init, view) };
    rune::run_with(spec, Host { mode, font, started, scene, venus: None, slot: [0.0; 4], frame: 0, generation: 1, last_size: (0, 0) });
}

// ---------------------------------------------------------------- capture -> Venus scene

fn style(fg: Color, bg: Option<Color>, bold: bool, italic: bool, faint: bool, underline: bool, strike: bool) -> DrawStyle {
    DrawStyle {
        foreground: fg,
        background: bg.unwrap_or(TERM_BG),
        underline_color: fg,
        bold,
        italic,
        faint,
        blink: false,
        invisible: false,
        strikethrough: strike,
        overline: false,
        selected: false,
        background_is_default: bg.is_none(),
        protected: false,
        underline: if underline { Underline::Single } else { Underline::None },
    }
}

fn xterm(n: u32) -> Color {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0), (205, 0, 0), (0, 205, 0), (205, 205, 0), (0, 0, 238), (205, 0, 205), (0, 205, 205), (229, 229, 229),
        (127, 127, 127), (255, 0, 0), (0, 255, 0), (255, 255, 0), (92, 92, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
    ];
    let (r, g, b) = match n {
        0..=15 => BASE[n as usize],
        16..=231 => {
            let n = n - 16;
            let level = |v: u32| if v == 0 { 0 } else { (55 + 40 * v) as u8 };
            (level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        _ => {
            let v = (8 + 10 * (n.min(255) - 232)) as u8;
            (v, v, v)
        }
    };
    Color { r, g, b }
}

/// A `tmux capture-pane -e -p` dump: text with SGR colour and attribute sequences, one line per
/// row, state carried across lines.
fn parse_capture(src: &str) -> Scene {
    let (mut fg, mut bg) = (None::<Color>, None::<Color>);
    let (mut bold, mut italic, mut faint, mut underline, mut strike, mut reverse) = (false, false, false, false, false, false);
    let mut content = Vec::with_capacity(ROWS);
    for line in src.lines().take(ROWS) {
        let mut cells = Vec::with_capacity(COLS);
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                if chars.next() != Some('[') {
                    continue;
                }
                let mut body = String::new();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        last = c;
                        break;
                    }
                    body.push(c);
                }
                if last != 'm' {
                    continue;
                }
                let codes: Vec<u32> = body.split(';').map(|v| v.parse().unwrap_or(0)).collect();
                let mut i = 0;
                while i < codes.len() {
                    let extended = |i: &mut usize| -> Option<Color> {
                        match codes.get(*i + 1) {
                            Some(2) if *i + 4 < codes.len() => {
                                *i += 4;
                                Some(Color { r: codes[*i - 2] as u8, g: codes[*i - 1] as u8, b: codes[*i] as u8 })
                            }
                            Some(5) if *i + 2 < codes.len() => {
                                *i += 2;
                                Some(xterm(codes[*i]))
                            }
                            _ => None,
                        }
                    };
                    match codes[i] {
                        0 => {
                            (fg, bg) = (None, None);
                            (bold, italic, faint, underline, strike, reverse) = (false, false, false, false, false, false);
                        }
                        1 => bold = true,
                        2 => faint = true,
                        3 => italic = true,
                        4 => underline = true,
                        7 => reverse = true,
                        9 => strike = true,
                        22 => (bold, faint) = (false, false),
                        23 => italic = false,
                        24 => underline = false,
                        27 => reverse = false,
                        29 => strike = false,
                        n @ 30..=37 => fg = Some(xterm(n - 30)),
                        38 => fg = extended(&mut i),
                        39 => fg = None,
                        n @ 40..=47 => bg = Some(xterm(n - 40)),
                        48 => bg = extended(&mut i),
                        49 => bg = None,
                        n @ 90..=97 => fg = Some(xterm(n - 90 + 8)),
                        n @ 100..=107 => bg = Some(xterm(n - 100 + 8)),
                        _ => {}
                    }
                    i += 1;
                }
                continue;
            }
            if cells.len() >= COLS {
                continue;
            }
            let (f, b) = if reverse { (bg.unwrap_or(TERM_BG), Some(fg.unwrap_or(TERM_FG))) } else { (fg.unwrap_or(TERM_FG), bg) };
            cells.push(DrawCell {
                width: CellWidth::Narrow,
                text: if c == ' ' { String::new() } else { c.to_string() },
                hyperlink: String::new(),
                style: style(f, b, bold, italic, faint, underline, strike),
            });
        }
        let blank = style(TERM_FG, None, false, false, false, false, false);
        cells.resize(COLS, DrawCell { width: CellWidth::Narrow, text: String::new(), hyperlink: String::new(), style: blank });
        content.push(DrawRow { wrapped: false, wrap_continuation: false, cells });
    }
    let blank = style(TERM_FG, None, false, false, false, false, false);
    content.resize(ROWS, DrawRow { wrapped: false, wrap_continuation: false, cells: vec![DrawCell { width: CellWidth::Narrow, text: String::new(), hyperlink: String::new(), style: blank }; COLS] });
    Scene {
        revision: 1,
        columns: COLS as u16,
        rows: ROWS as u16,
        screen: Screen::Alternate,
        title: "nvim".into(),
        working_directory: String::new(),
        background: TERM_BG,
        foreground: TERM_FG,
        cursor: Some(DrawCursor {
            visible: true, blinking: false, password_input: false, shape: CursorShape::Block,
            column: 101, row: 29, at_wide_tail: false, color: TERM_FG, explicit_color: false,
        }),
        content,
        graphics: SceneGraphics::default(),
    }
}

fn hash(i: u64, generation: u64) -> u64 {
    let mut h = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ generation.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 29;
    h.wrapping_mul(0xBF58_476D_1CE4_E5B9)
}

/// A new glyph and colours for one cell: any printable ASCII, a hashed foreground, and a
/// background on one cell in four.
fn scramble(cell: &mut DrawCell, i: usize, generation: u64) {
    let h = hash(i as u64, generation);
    cell.text.clear();
    cell.text.push((b'!' + (h % 94) as u8) as char);
    let fg = Color { r: 96 | (h >> 8) as u8, g: 96 | (h >> 16) as u8, b: 96 | (h >> 24) as u8 };
    let bg = (h >> 32) % 4 == 0;
    cell.style = style(fg, bg.then_some(Color { r: (h >> 40) as u8 & 0x3f, g: (h >> 48) as u8 & 0x3f, b: (h >> 56) as u8 & 0x3f }), h >> 34 & 7 == 0, false, false, false, false);
}

// ---------------------------------------------------------------- hosting Venus in the hook

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Static,
    One,
    All,
}

struct Host {
    mode: Mode,
    font: f32,
    started: Instant,
    scene: Scene,
    venus: Option<(DeviceKey, wgpu::TextureFormat, Renderer)>,
    slot: [f32; 4],
    frame: u64,
    generation: u64,
    last_size: (u32, u32),
}

impl Compositor<State> for Host {
    fn render(&mut self, f: &mut Frame<'_, State>) {
        let entered = Instant::now();
        let (device, queue, clear, key) = (f.device, f.queue, f.rt.clear_color(), f.device_key());
        let first = self.frame == 0;
        if first {
            let epoch_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
            let info = device.adapter_info();
            eprintln!("hosted: first render at epoch_ms {epoch_ms}, {:.1} ms after main; adapter {:?} backend {:?} type {:?}; target {:?} {:?} srgb_blend {}",
                self.started.elapsed().as_secs_f64() * 1000.0, info.name, info.backend, info.device_type, f.size, f.format, f.srgb_blend());
        }
        if f.size != self.last_size {
            eprintln!("hosted: frame {} target size {:?} -> {:?}", self.frame, self.last_size, f.size);
            self.last_size = f.size;
        }
        // The slot's record is in the instance list before the pass opens: Venus needs its size
        // to prepare, and the hook only sees it once the pass is open.
        let slot = f.rt.instances.iter().find(|i| i.params[1] == KIND_EFFECT).map(|i| (i.rect, i.params[3]));
        eprintln!("resize-proof: frame {} size {:?} viewport {:?} scale {} slot {:?}", self.frame, f.size, f.rt.window_viewport(), f.rt.window_scale(), slot);
        if let Some((rect, scale)) = slot {
            let size = (rect[2].round().max(1.0) as u32, rect[3].round().max(1.0) as u32);
            if rect != self.slot {
                eprintln!("hosted: frame {} slot rect {:?} -> {:?} scale {scale}", self.frame, self.slot, rect);
            }
            if self.venus.as_ref().is_none_or(|(k, format, _)| *k != key || *format != f.format) {
                let fonts = FontSetup::new(&FontSettings { size: self.font, ..FontSettings::default() }).expect("venus fonts");
                let venus = Renderer::hosted(device.clone(), queue.clone(), f.format, size, f64::from(scale), fonts).expect("venus renderer");
                let m = venus.metrics();
                eprintln!("hosted: venus renderer made for {:?} at {:?}; cell {}x{} padding {} font {} -> grid needs {}x{}",
                    f.format, size, m.width, m.height, m.padding, m.font_size, COLS as f32 * m.width + 2.0 * m.padding, ROWS as f32 * m.height + 2.0 * m.padding);
                self.venus = Some((key, f.format, venus));
            } else if rect[2..] != self.slot[2..] {
                self.venus.as_mut().unwrap().2.resize(PhysicalSize::new(size.0, size.1), f64::from(scale));
            }
            self.slot = rect;
            if !first {
                match self.mode {
                    Mode::Static => {}
                    Mode::One => {
                        let i = (self.frame as usize * 7919) % (COLS * ROWS);
                        scramble(&mut self.scene.content[i / COLS].cells[i % COLS], i, self.frame);
                        self.generation += 1;
                    }
                    Mode::All => {
                        for (r, row) in self.scene.content.iter_mut().enumerate() {
                            for (c, cell) in row.cells.iter_mut().enumerate() {
                                scramble(cell, r * COLS + c, self.frame);
                            }
                        }
                        self.generation += 1;
                    }
                }
                self.scene.revision = self.generation;
            }
            let before = Instant::now();
            if let Err(e) = self.venus.as_mut().unwrap().2.prepare_hosted(&self.scene, true, self.generation) {
                eprintln!("hosted: venus prepare failed: {e}");
            }
            PREP_NS.fetch_add(before.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        f.ui.prepare(device, queue, &mut f.rt.text.atlas, &f.rt.instances, f.size);
        let venus = self.venus.as_ref().map(|v| &v.2);
        f.ui.draw_with(f.encoder, f.target, clear, &mut |pass, site| {
            if let (GRID_MARK, Some(venus)) = (site.mark, venus) {
                // Venus draws as if its target were the slot: the pass viewport is its sub-rect.
                let r = site.instance.rect;
                pass.set_viewport(r[0] - site.origin[0], r[1] - site.origin[1], r[2], r[3], 0.0, 1.0);
                if let Err(e) = venus.draw_hosted(pass) {
                    eprintln!("hosted: venus draw failed: {e}");
                }
            }
        });
        if let Some((_, _, venus)) = self.venus.as_mut() {
            venus.end_frame_hosted();
        }
        self.frame += 1;
        FRAMES.fetch_add(1, Ordering::Relaxed);
        RENDER_NS.fetch_add(entered.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    fn animating(&self) -> bool {
        self.mode != Mode::Static || self.venus.as_ref().is_some_and(|v| v.2.cursor_animation_active())
    }
}
