//! Re-scoped Eon transition gate. Same virtual empty-terminal host as E.
//! F6 selects, F7 zooms/restores, F8 shows/hides a popup, F9 switches tabs.
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rune::compose::{Compositor, DeviceKey, Frame};
use rune::machine::Effects;
use rune::render::{KIND_EFFECT, KIND_LAYER};
use rune::tok::{self, type_};
use rune::{
    App, AppSpec, Cx, El, Key, KeyEvent, TextEdit, Tx, Waker, col, effect, el, row, text,
    text_input,
};
use winit::dpi::PhysicalSize;
use yazelix_venus::{FontSettings, FontSetup, Renderer, Scene};

#[path = "terminal/pty.rs"]
mod pty;
mod terminal;
use pty::Terminal;
use terminal::{encode, scene_from};

const KEYS: [&str; 2] = ["pane-0", "pane-1"];
static WAKER: OnceLock<Waker> = OnceLock::new();
static FRAMES: AtomicU64 = AtomicU64::new(0);
static PREP_NS: AtomicU64 = AtomicU64::new(0);
static RENDER_NS: AtomicU64 = AtomicU64::new(0);
static PREPARES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

enum State {
    Split,
    Solo(usize),
}

impl State {
    fn visible(&self, pane: usize) -> bool {
        match self {
            Self::Split => true,
            Self::Solo(active) => *active == pane,
        }
    }
}

enum Event {
    Split,
    Solo(usize),
    Focus(usize),
}

enum Effect {
    Focus(usize),
}

fn step(state: &mut State, event: Event) -> Effects<Effect> {
    match event {
        Event::Split => {
            *state = State::Split;
            Effects::none()
        }
        Event::Solo(pane) => {
            *state = State::Solo(pane);
            Effects::one(Effect::Focus(pane))
        }
        Event::Focus(pane) => {
            if matches!(state, State::Solo(_)) {
                *state = State::Split;
            }
            Effects::one(Effect::Focus(pane))
        }
    }
}

struct AppState {
    mode: State,
    popup: bool,
    tab: usize,
    terminals: Vec<Terminal>,
    active: usize,
    focused: Option<usize>,
    window_focused: bool,
    seen: [u64; 2],
    field: TextEdit,
}

fn dispatch(s: &mut AppState, event: Event, tx: &mut Tx) {
    for Effect::Focus(pane) in step(&mut s.mode, event) {
        assert!(pane < KEYS.len());
        s.active = pane;
        tx.focus(KEYS[pane], false);
    }
    for (index, terminal) in s.terminals.iter().enumerate() {
        terminal.set_visible(s.mode.visible(index));
    }
    eprintln!(
        "panes: layout {} active {}",
        if matches!(s.mode, State::Solo(_)) {
            "solo"
        } else {
            "split"
        },
        s.active
    );
}

fn pane_key(tx: &Tx) -> Option<usize> {
    KEYS.iter().position(|key| Some(*key) == tx.focused())
}

fn terminal_key(s: &mut AppState, event: &KeyEvent, tx: &mut Tx) -> bool {
    if matches!(event.key, Key::Function(6 | 7 | 8 | 9 | 12)) {
        return false;
    }
    let terminals = &s.terminals;
    if terminals.is_empty() {
        return false;
    }
    if event.key == Key::Command('v') && event.mods.shift {
        tx.request_paste();
        return true;
    }
    let pane = pane_key(tx).unwrap_or(s.active);
    let application_cursor = terminals[pane]
        .screen()
        .parser
        .screen()
        .application_cursor();
    let Some(bytes) = encode(event, application_cursor) else {
        return false;
    };
    terminals[pane].write(&bytes);
    true
}

fn on_key(s: &mut AppState, event: &KeyEvent, tx: &mut Tx) -> bool {
    match event.key {
        Key::Function(6) => dispatch(s, Event::Focus(1 - s.active), tx),
        Key::Function(7) => {
            let event = if matches!(s.mode, State::Solo(_)) {
                Event::Split
            } else {
                Event::Solo(s.active)
            };
            dispatch(s, event, tx);
            tx.focus(KEYS[s.active], false);
        }
        Key::Function(8) => {
            s.popup = !s.popup;
            tx.focus(if s.popup { "popup" } else { KEYS[s.active] }, false);
        }
        Key::Function(9) => {
            s.tab = 1 - s.tab;
            s.popup = false;
            dispatch(s, Event::Split, tx);
            tx.focus(KEYS[s.active], false);
        }
        Key::Function(12) => tx.focus(
            if pane_key(tx).is_some() {
                "field"
            } else {
                KEYS[s.active]
            },
            false,
        ),
        _ => return false,
    }
    // Venus changes these layouts at once; the gate measures the actual changed frame.
    tx.instant();
    true
}

fn chip(label: &str, theme: &tok::Theme) -> El {
    el().px(8.0)
        .py(3.0)
        .radius(5.0)
        .fill(theme.card)
        .child(text(label).font(type_::LABEL).color(theme.text))
}

fn tab(label: &'static str, selected: bool, event: fn(usize) -> Event) -> El {
    let theme = tok::theme(true);
    el().key(label)
        .focusable(true)
        .role("tab")
        .label(label)
        .checked(selected)
        .px(12.0)
        .py(7.0)
        .radius(7.0)
        .fill(if selected { theme.card } else { theme.bg })
        .hover(theme.control_off)
        .child(text(label).font(type_::LABEL).color(if selected {
            theme.text
        } else {
            theme.text_secondary
        }))
        .on_press_tx(move |s: &mut AppState, tx| dispatch(s, event(s.active), tx))
}

fn pane(s: &AppState, index: usize) -> El {
    let theme = tok::theme(true);
    let active = s.focused == Some(index) && s.window_focused;
    let header = row()
        .h(30.0)
        .px(12.0)
        .items_center()
        .fill(theme.card)
        .child(
            text(format!("Tab {} / pane {}  bash", s.tab + 1, index + 1))
                .font(type_::LABEL)
                .color(theme.text),
        )
        .on_press_tx(move |s: &mut AppState, tx| dispatch(s, Event::Focus(index), tx));
    col()
        .key(format!("tab-{}-shell-{index}", s.tab))
        .motion(rune::Spring::SMOOTH)
        .transition(rune::Transition::none())
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .border(
            1.0,
            if active {
                theme.text_secondary
            } else {
                theme.card
            },
        )
        .radius(7.0)
        .clip_soft(7.0)
        .child(header)
        .child(
            effect(index as u32 + 1)
                .effect_data([index as f32, 0.0, 0.0, 0.0])
                .key(KEYS[index])
                .w_full()
                .grow(1.0)
                .focusable(true)
                .label(format!("Terminal {}", index + 1))
                .on_key(terminal_key)
                .on_press_tx(move |s: &mut AppState, tx| dispatch(s, Event::Focus(index), tx)),
        )
}

fn view(s: &AppState, _: &mut Cx<AppState>) -> El {
    let theme = tok::theme(true);
    let tabs = row()
        .h(42.0)
        .px(8.0)
        .gap(4.0)
        .items_center()
        .role("tablist")
        .child(tab("Split", matches!(s.mode, State::Split), |_| {
            Event::Split
        }))
        .child(tab(
            "Focused pane",
            matches!(s.mode, State::Solo(_)),
            Event::Solo,
        ));
    let mut panes = row().w_full().grow(1.0).padding(8.0).gap(8.0);
    match s.mode {
        State::Solo(index) => panes = panes.child(pane(s, index)),
        State::Split => panes = panes.child(pane(s, 0)).child(pane(s, 1)),
    }
    if s.popup {
        panes = panes.child(
            col()
                .key("popup")
                .transition(rune::Transition::none())
                .pin(Some(8.0), Some(8.0), Some(8.0), Some(8.0))
                .focusable(true)
                .label("Files popup")
                .fill(theme.card)
                .child(text("Files popup").font(type_::LABEL).color(theme.text)),
        );
    }
    // shortcut: resource and quota labels are fixtures; wire real readings during a product port.
    let bar = row()
        .h(36.0)
        .w_full()
        .px(8.0)
        .gap(6.0)
        .items_center()
        .fill(theme.bg)
        .child(chip("14:32", &theme))
        .child(chip("eon 0.0.0", &theme))
        .child(chip("CPU 12%", &theme))
        .child(chip("RAM 9.1G", &theme))
        .child(chip("disk 61%", &theme))
        .child(chip("quota 72%", &theme))
        // The trace marker reads poll's actual runtime focus, not the requested target.
        .child(
            el().key(
                s.focused
                    .map_or("focus-none".to_owned(), |p| format!("focus-pane-{p}")),
            )
            .transition(rune::Transition::none())
            .w(2.0)
            .h(2.0)
            .fill(theme.text),
        )
        .child(el().grow(1.0))
        .child(
            text_input(&s.field, type_::BODY)
                .key("field")
                .single_line()
                .w(180.0)
                .px(8.0)
                .py(4.0)
                .radius(5.0)
                .fill(theme.card)
                .color(theme.text)
                .placeholder("Search", theme.text_secondary)
                .on_edit(|s: &mut AppState| &mut s.field),
        );
    col()
        .w_full()
        .h_full()
        .fill(theme.bg)
        .child(tabs)
        .child(panes)
        .child(bar)
        .on_paste(|s: &mut AppState, pasted: &str, tx| {
            let terminals = &s.terminals;
            if terminals.is_empty() {
                return;
            }
            let pane = pane_key(tx).unwrap_or(s.active);
            let bracketed = terminals[pane].screen().parser.screen().bracketed_paste();
            let bytes = if bracketed {
                format!("\x1b[200~{pasted}\x1b[201~")
            } else {
                pasted.to_owned()
            };
            terminals[pane].write(bytes.as_bytes());
            eprintln!("panes: paste pane {pane} bytes {}", pasted.len());
        })
}

fn poll(s: &mut AppState, _: f64, tx: &mut Tx) -> bool {
    let focused = pane_key(tx);
    let mut changed = focused != s.focused;
    s.focused = focused;
    if let Some(pane) = focused {
        s.active = pane;
    }
    for (index, terminal) in s.terminals.iter().enumerate() {
        let generation = terminal.screen().generation;
        changed |= s.mode.visible(index) && generation != s.seen[index];
        s.seen[index] = generation;
    }
    changed
}

fn main() {
    let started = Instant::now();
    if let Ok(path) = std::env::var("TERMGRID_STATS") {
        std::thread::spawn(move || {
            let mut file = std::fs::File::create(path).unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                writeln!(
                    file,
                    "{:.3} {} {:.3} {:.3} {} {}",
                    started.elapsed().as_secs_f64(),
                    FRAMES.load(Ordering::Relaxed),
                    PREP_NS.load(Ordering::Relaxed) as f64 / 1e6,
                    RENDER_NS.load(Ordering::Relaxed) as f64 / 1e6,
                    PREPARES[0].load(Ordering::Relaxed),
                    PREPARES[1].load(Ordering::Relaxed)
                )
                .unwrap();
            }
        });
    }
    let spec = AppSpec {
        setup: Some(|_, wake| {
            let _ = WAKER.set(wake);
        }),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new(
            "eon-transitions",
            (1920.0, 1080.0),
            || AppState {
                mode: State::Solo(0),
                popup: false,
                tab: 0,
                terminals: Vec::new(),
                active: 0,
                focused: None,
                window_focused: true,
                seen: [0; 2],
                field: TextEdit::new(""),
            },
            view,
        )
    };
    let app = App::new(spec).compositor(Host {
        panes: std::array::from_fn(|_| [None, None]),
        started,
    });
    let handle = app.handle();
    app.on_ready(move |_| {
        handle.update(|s, tx| {
            s.terminals = (0..2)
                .map(|pane| Terminal::spawn(pane, WAKER.get().unwrap().clone()))
                .collect();
            for (index, terminal) in s.terminals.iter().enumerate() {
                terminal.set_visible(s.mode.visible(index));
            }
            tx.focus(KEYS[0], false);
            tx.instant();
        })
    })
    .on_focus(|s, focused, _| s.window_focused = focused)
    .run();
}

struct PaneRenderer {
    device: DeviceKey,
    format: wgpu::TextureFormat,
    renderer: Renderer,
    rect: [f32; 4],
    scale: f32,
    scene: Option<Scene>,
    prepared: Option<(u64, bool)>,
}

struct Host {
    // shortcut: per-format renderers duplicate fonts; a port should share text across formats.
    panes: [[Option<PaneRenderer>; 2]; 2],
    started: Instant,
}

impl Compositor<AppState> for Host {
    fn render(&mut self, frame: &mut Frame<'_, AppState>) {
        let entered = Instant::now();
        let key = frame.device_key();
        if FRAMES.load(Ordering::Relaxed) == 0 {
            eprintln!(
                "panes: first render {:.1} ms, adapter {:?}",
                self.started.elapsed().as_secs_f64() * 1000.0,
                frame.device.adapter_info().name
            );
        }
        let formats = [frame.format, wgpu::TextureFormat::Rgba16Float];
        let layers = frame.rt.instances.iter().any(|i| i.params[1] == KIND_LAYER);
        for slot in frame
            .rt
            .instances
            .iter()
            .filter(|i| i.params[1] == KIND_EFFECT)
        {
            let index = slot.uv[0] as usize;
            assert!(index < self.panes.len());
            let Some(terminal) = frame.rt.state.terminals.get(index) else {
                continue;
            };
            let size = PhysicalSize::new(
                slot.rect[2].round().max(1.0) as u32,
                slot.rect[3].round().max(1.0) as u32,
            );
            let scale = slot.params[3];
            for (variant, format) in formats.iter().copied().enumerate() {
                if variant == 1 && (!layers || format == frame.format) {
                    continue;
                }
                if self.panes[index][variant]
                    .as_ref()
                    .is_none_or(|pane| pane.device != key || pane.format != format)
                {
                    let fonts = FontSetup::new(&FontSettings {
                        size: 14.0,
                        ..FontSettings::default()
                    })
                    .expect("fonts");
                    self.panes[index][variant] = Some(PaneRenderer {
                        device: key,
                        format,
                        renderer: Renderer::hosted(
                            frame.device.clone(),
                            frame.queue.clone(),
                            format,
                            (size.width, size.height),
                            f64::from(scale),
                            fonts,
                        )
                        .expect("hosted renderer"),
                        rect: slot.rect,
                        scale,
                        scene: None,
                        prepared: None,
                    });
                }
                let pane = self.panes[index][variant].as_mut().unwrap();
                if pane.rect[2..] != slot.rect[2..] || pane.scale != scale {
                    pane.renderer.resize(size, f64::from(scale));
                    pane.prepared = None;
                }
                pane.rect = slot.rect;
                pane.scale = scale;
                let metrics = pane.renderer.metrics();
                let columns = ((slot.rect[2] - 2.0 * metrics.padding) / metrics.width)
                    .floor()
                    .clamp(2.0, 500.0) as u16;
                let rows = ((slot.rect[3] - 2.0 * metrics.padding) / metrics.height)
                    .floor()
                    .clamp(2.0, 300.0) as u16;
                terminal.resize(columns, rows);
                let focused =
                    frame.rt.state.focused == Some(index) && frame.rt.state.window_focused;
                let terminal = terminal.screen();
                let snapshot = (terminal.generation, focused);
                if pane.prepared != Some(snapshot) {
                    pane.scene = Some(scene_from(
                        terminal.parser.screen(),
                        terminal.generation,
                        focused,
                    ));
                    pane.prepared = Some(snapshot);
                    PREPARES[index].fetch_add(1, Ordering::Relaxed);
                }
                let generation = terminal.generation * 2 + u64::from(focused);
                drop(terminal);
                let before = Instant::now();
                pane.renderer
                    .prepare_hosted(pane.scene.as_ref().unwrap(), true, generation)
                    .expect("Venus prepare");
                PREP_NS.fetch_add(before.elapsed().as_nanos() as u64, Ordering::Relaxed);
            }
        }
        frame.ui.prepare(
            frame.device,
            frame.queue,
            &mut frame.rt.text.atlas,
            &frame.rt.instances,
            frame.size,
        );
        let clear = frame.rt.clear_color();
        frame
            .ui
            .draw_with(frame.encoder, frame.target, clear, &mut |pass, site| {
                if let Some(pane) = site
                    .mark
                    .checked_sub(1)
                    .and_then(|i| self.panes.get(i as usize))
                    .and_then(|variants| {
                        variants.iter().flatten().find(|p| p.format == site.format)
                    })
                {
                    let rect = site.instance.rect;
                    pass.set_viewport(
                        rect[0] - site.origin[0],
                        rect[1] - site.origin[1],
                        rect[2],
                        rect[3],
                        0.0,
                        1.0,
                    );
                    pane.renderer.draw_hosted(pass).expect("Venus draw");
                }
            });
        for pane in self.panes.iter_mut().flatten().flatten() {
            pane.renderer.end_frame_hosted();
        }
        FRAMES.fetch_add(1, Ordering::Relaxed);
        RENDER_NS.fetch_add(entered.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    fn animating(&self) -> bool {
        self.panes
            .iter()
            .flatten()
            .flatten()
            .any(|pane| pane.renderer.cursor_animation_active())
    }
}
