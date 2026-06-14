pub mod attach;

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::sync::mpsc;

use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tracing::debug;

/// RAII guard that restores terminal state on drop.
struct TerminalGuard {
    original: libc::termios,
    fd: i32,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        let fd = io::stdin().as_raw_fd();
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        unsafe {
            if libc::tcgetattr(fd, &mut original) != 0 {
                anyhow::bail!("tcgetattr failed");
            }
        }

        let mut raw = original;
        raw.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
        raw.c_oflag = 0;
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
        raw.c_cflag &= !(libc::CSIZE | libc::PARENB);
        raw.c_cflag |= libc::CS8;
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;

        unsafe {
            libc::tcsetattr(fd, libc::TCSAFLUSH, &raw);
        }

        Ok(TerminalGuard { original, fd })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.original);
        }
    }
}

/// Run a command in a foreground PTY attached to the current terminal.
///
/// **EXPERIMENTAL — UNUSED IN NORMAL FLOW.**
/// This is the foreground fallback path used when the daemon is unavailable.
/// It does not support attach/detach. The daemon path is the primary interface.
/// This code is retained for potential future use but is not called by any
/// CLI command path. It uses an older SIGWINCH shutdown style (mpsc channel)
/// rather than the safer `Handle::close()` pattern used in `attach.rs`.
#[allow(dead_code)]
pub fn run_foreground(
    command: &str,
    args: &[String],
    cwd: Option<&str>,
    initial_size: PtySize,
) -> Result<i32> {
    let pty_system = native_pty_system();

    let pair = pty_system.openpty(initial_size)?;

    let mut cmd = CommandBuilder::new(command);
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.cwd(cwd);
    }

    let mut child = pair.slave.spawn_command(cmd)?;

    let writer = pair.master.take_writer()?;
    let reader = pair.master.try_clone_reader()?;

    // Pipe PTY output → stdout
    let output_thread = std::thread::spawn(move || {
        let mut stdout = io::stdout();
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = stdout.write_all(&buf[..n]);
                    let _ = stdout.flush();
                }
                Err(_) => break,
            }
        }
    });

    // Set raw mode on stdin
    let _guard = TerminalGuard::new();

    // SIGWINCH → resize PTY.
    let (tx_shutdown, rx_shutdown) = mpsc::channel::<()>();

    let resize_thread = std::thread::spawn(move || {
        let mut sig = match signal_hook::iterator::Signals::new([signal_hook::consts::SIGWINCH]) {
            Ok(s) => s,
            Err(_) => return,
        };

        for _ in sig.forever() {
            if rx_shutdown.try_recv().is_ok() {
                return;
            }
            if let Some((rows, cols)) = terminal_size() {
                let _ = pair.master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
    });

    // Pipe stdin → PTY
    let writer_thread = std::thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut writer = writer;
        let mut buf = [0u8; 1024];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                Err(_) => break,
            }
        }
    });

    // Wait for child to exit
    let status = child.wait()?;
    debug!("child exited: {:?}", status);

    // Signal resize thread to stop
    let _ = tx_shutdown.send(());

    // Wait for threads
    let _ = output_thread.join();
    let _ = writer_thread.join();
    let _ = resize_thread.join();

    Ok(status.exit_code() as i32)
}

/// Get terminal size (rows, cols) via ioctl TIOCGWINSZ.
fn terminal_size() -> Option<(u16, u16)> {
    #[repr(C)]
    struct WinSize {
        ws_row: u16,
        ws_col: u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }

    let fd = io::stdout().as_raw_fd();
    let mut ws = WinSize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    const TIOCGWINSZ: libc::c_ulong = 0x5413;
    let ret = unsafe { libc::ioctl(fd, TIOCGWINSZ, &mut ws as *mut _) };
    if ret == 0 && ws.ws_row > 0 && ws.ws_col > 0 {
        Some((ws.ws_row, ws.ws_col))
    } else {
        None
    }
}

/// Detect current terminal size, fallback to 24x80.
#[allow(dead_code)]
pub fn detect_terminal_size() -> PtySize {
    terminal_size()
        .map(|(rows, cols)| PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap_or(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
}
