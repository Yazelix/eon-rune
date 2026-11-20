//! A Linux build without the `x11` feature runs on Wayland alone. Without a Wayland display,
//! `try_run` refuses before any windowing work, X11 offered or not, and says why; with a Wayland
//! socket that doesn't exist, winit's own Wayland connect error comes back. Each case runs in a
//! child process with a clean environment, so the test runner's display never leaks in. The X
//! display offered to the child is `RUNE_WAYLAND_ONLY_X_DISPLAY` (a harness's private Xvfb, which
//! watches for connections), else `:4242`, a display number nothing normally serves.

#[cfg(not(all(target_os = "linux", not(feature = "x11"))))]
fn main() { println!("wayland_only: skipped (x11 on)"); }

#[cfg(all(target_os = "linux", not(feature = "x11")))]
fn main() { wayland::main() }

#[cfg(all(target_os = "linux", not(feature = "x11")))]
mod wayland {
    use rune::{App, AppSpec, Cx, El, text};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const CHILD: &str = "RUNE_WAYLAND_ONLY_CHILD";
    const REFUSAL: &str = "event loop: no Wayland display (WAYLAND_DISPLAY and WAYLAND_SOCKET are unset); this build of Rune has no X11 support";

    fn view(_: &(), _: &mut Cx<()>) -> El { text("never shown") }

    pub fn main() {
        if std::env::var_os(CHILD).is_some() {
            let result = App::new(AppSpec::new("wayland_only", (100.0, 100.0), || (), view)).args(Vec::new()).try_run();
            match result {
                Ok(()) => println!("RESULT ok"),
                Err(e) => println!("RESULT err {e}"),
            }
            return;
        }
        let runtime = std::env::temp_dir().join(format!("rune-wayland-only-{}", std::process::id()));
        std::fs::create_dir_all(&runtime).unwrap_or_else(|e| panic!("{}: {e}", runtime.display()));
        // An X server name that the child could reach if it tried X11, and one Wayland socket
        // name with nothing behind it.
        let x = std::env::var_os("RUNE_WAYLAND_ONLY_X_DISPLAY").map_or_else(|| ":4242".to_owned(), |d| d.to_string_lossy().into_owned());
        let refusal = child(&runtime, &[("DISPLAY", &x)]);
        assert_eq!(refusal, format!("RESULT err {REFUSAL}"), "no Wayland display");
        println!("wayland_only: case 1 ok");
        let missing = child(&runtime, &[("WAYLAND_DISPLAY", "no-such-wayland-socket"), ("DISPLAY", &x)]);
        // winit's Wayland backend reports it (its `os_error!` location), not an X11 fallback.
        assert!(missing.starts_with("RESULT err event loop:") && missing.contains("platform_impl/linux/wayland/"), "a missing Wayland socket: {missing}");
        assert!(!missing.contains(REFUSAL), "a Wayland variable set: winit tries it: {missing}");
        println!("wayland_only: case 2 ok");
        std::fs::remove_dir_all(&runtime).unwrap_or_else(|e| panic!("{}: {e}", runtime.display()));
    }

    /// Runs this test binary as the child with only `vars` beyond a minimal environment, and
    /// returns its `RESULT` line. Fails after 20 s.
    fn child(runtime: &std::path::Path, vars: &[(&str, &str)]) -> String {
        let mut command = Command::new(std::env::current_exe().unwrap_or_else(|e| panic!("this test's path: {e}")));
        command.env_clear().env(CHILD, "1").env("XDG_RUNTIME_DIR", runtime).stdout(Stdio::piped()).stderr(Stdio::piped());
        // LD_LIBRARY_PATH: winit dlopens libwayland-client, which a Nix shell finds only there.
        for name in ["PATH", "HOME", "LD_LIBRARY_PATH"] {
            if let Some(value) = std::env::var_os(name) { command.env(name, value); }
        }
        command.envs(vars.iter().copied());
        let mut process = command.spawn().unwrap_or_else(|e| panic!("spawning the child: {e}"));
        let start = Instant::now();
        while process.try_wait().unwrap_or_else(|e| panic!("waiting for the child: {e}")).is_none() {
            if start.elapsed() > Duration::from_secs(20) {
                drop(process.kill());
                panic!("the child did not finish within 20 s");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let output = process.wait_with_output().unwrap_or_else(|e| panic!("the child's output: {e}"));
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.lines().find(|line| line.starts_with("RESULT ")).map(str::to_owned)
            .unwrap_or_else(|| panic!("no RESULT line; stdout: {stdout}; stderr: {}", String::from_utf8_lossy(&output.stderr)))
    }
}
