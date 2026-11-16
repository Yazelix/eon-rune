//! One live PTY, its bounded vt100 screen, and its output wakeup.
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};

pub struct Screen {
    pub parser: vt100::Parser,
    pub generation: u64,
    visible: bool,
}

pub struct Terminal {
    screen: Arc<Mutex<Screen>>,
    master: std::fs::File,
}

impl Terminal {
    pub fn spawn(pane: usize, wake: rune::Waker) -> Self {
        let size = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let (mut master, mut slave) = (0, 0);
        // SAFETY: valid output pointers and winsize; fresh descriptors are owned below.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &size,
                )
            },
            0
        );
        // SAFETY: descriptors returned by successful openpty.
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        // SAFETY: fcntl only changes flags on the owned descriptor.
        assert!(unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } >= 0);
        let mut command = Command::new("bash");
        command
            .args(["--norc", "--noprofile", "-i"])
            .env("TERM", "xterm-256color")
            .env("PS1", "spike$ ")
            .env("TERMGRID_PANE", pane.to_string())
            .env("HISTFILE", "/dev/null")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe operations in the child before exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // Background harness launches otherwise pass ignored Ctrl+C to shell jobs.
                for sig in [
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGTSTP,
                    libc::SIGTTIN,
                    libc::SIGTTOU,
                    libc::SIGCHLD,
                    libc::SIGPIPE,
                ] {
                    if libc::signal(sig, libc::SIG_DFL) == libc::SIG_ERR {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = command.spawn().expect("PTY shell");
        eprintln!("panes: shell pane {pane} pid {}", child.id());
        let master = std::fs::File::from(master);
        let mut reader = master.try_clone().unwrap();
        let screen = Arc::new(Mutex::new(Screen {
            parser: vt100::Parser::new(
                NonZeroU16::new(24).unwrap(),
                NonZeroU16::new(80).unwrap(),
                0,
            ),
            generation: 1,
            visible: true,
        }));
        let output = screen.clone();
        std::thread::spawn(move || {
            let mut bytes = [0; 1 << 16];
            loop {
                let count = match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => count,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        eprintln!("panes: PTY {pane} read ended: {error}");
                        break;
                    }
                };
                let visible = {
                    let mut screen = output.lock().unwrap();
                    screen.parser.process(&bytes[..count]);
                    screen.generation += 1;
                    screen.visible
                };
                if visible {
                    wake();
                }
            }
            eprintln!("panes: shell pane {pane} exited {:?}", child.wait());
        });
        Self { screen, master }
    }

    pub fn screen(&self) -> MutexGuard<'_, Screen> {
        self.screen.lock().unwrap()
    }

    pub fn set_visible(&self, visible: bool) {
        self.screen().visible = visible;
    }

    pub fn write(&self, bytes: &[u8]) {
        (&self.master).write_all(bytes).expect("PTY write");
    }

    pub fn resize(&self, columns: u16, rows: u16) {
        let (columns, rows) = (
            NonZeroU16::new(columns).unwrap(),
            NonZeroU16::new(rows).unwrap(),
        );
        let mut screen = self.screen();
        if screen.parser.screen().size() == (rows, columns) {
            return;
        }
        let size = libc::winsize {
            ws_row: rows.get(),
            ws_col: columns.get(),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: TIOCSWINSZ on the owned PTY master with a valid winsize.
        assert_eq!(
            unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) },
            0
        );
        screen.parser.screen_mut().set_size(rows, columns);
        screen.generation += 1;
    }
}
