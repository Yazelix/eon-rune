//! #372: an immediately applied Wayland resize reaches the hook and viewport without input.
//! Run with RUNE_GUI_TESTS=1 on a private Wayland display.
use rune::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Geometry = ((u32, u32), (f32, f32), f32);

struct Hook(Arc<Mutex<Option<Geometry>>>);

impl Compositor<()> for Hook {
    fn render(&mut self, frame: &mut Frame<'_, ()>) {
        *self.0.lock().unwrap() = Some((
            frame.size,
            frame.rt.window_viewport(),
            frame.rt.window_scale(),
        ));
        frame.draw_ui();
    }
}

fn matches((physical, viewport, scale): Geometry, size: (f32, f32)) -> bool {
    let pixel = 1.0 / scale;
    (viewport.0 - size.0).abs() <= pixel
        && (viewport.1 - size.1).abs() <= pixel
        && physical
            == (
                (size.0 * scale).round() as u32,
                (size.1 * scale).round() as u32,
            )
}

fn wait_for(observed: &Mutex<Option<Geometry>>, size: (f32, f32)) {
    let start = rune::time::now();
    loop {
        let got = *observed.lock().unwrap();
        if got.is_some_and(|got| matches(got, size)) {
            println!("self_resize: geometry {got:?}");
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "resize to {size:?}: hook/viewport/scale {got:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn main() {
    assert!(matches(
        ((1080, 720), (1080.0 / 1.2, 720.0 / 1.2), 1.2),
        (900.0, 600.0)
    ));
    assert!(!matches(
        ((1200, 720), (1200.0, 720.0), 1.0),
        (900.0, 600.0)
    ));
    assert!(!matches(((1080, 720), (902.0, 600.0), 1.2), (900.0, 600.0)));
    if !cfg!(target_os = "linux")
        || std::env::var_os("RUNE_GUI_TESTS").is_none()
        || std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        println!("self_resize: skipped (set RUNE_GUI_TESTS=1 on a private Wayland display)");
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(20));
        eprintln!("self_resize: timeout");
        std::process::exit(2);
    });
    let observed = Arc::new(Mutex::new(None));
    let (ready, opened) = std::sync::mpsc::channel();
    let app = App::new(AppSpec::new(
        "self resize",
        (1200.0, 720.0),
        || (),
        |_, _| col().w_full().h_full().fill(Color::BLACK),
    ))
    .args(Vec::new())
    .float_on_open()
    .on_ready(move |_| {
        ready.send(()).unwrap();
    })
    .compositor(Hook(observed.clone()));
    let handle = app.handle();
    let passed = Arc::new(Mutex::new(false));
    let result = passed.clone();
    std::thread::spawn(move || {
        let run = std::panic::catch_unwind(|| {
            opened
                .recv_timeout(Duration::from_secs(5))
                .expect("window ready");
            wait_for(&observed, (1200.0, 720.0));
            for size in [(900.0, 600.0), (1200.0, 720.0)] {
                handle.update(move |_, tx| tx.resize_window_instant(size));
                wait_for(&observed, size);
            }
        });
        *result.lock().unwrap() = run.is_ok();
        handle.close();
    });
    rune::orderly_exit();
    app.run();
    assert!(*passed.lock().unwrap(), "self_resize failed");
    println!("self_resize: ok");
}
