use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use anyhow::Result;
use signal_hook::iterator::{Handle, Signals};

use crate::daemon::server;

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

fn send_current_size(session_name: &str) {
    if let Some((rows, cols)) = terminal_size() {
        let _ = server::send_resize(session_name, rows, cols);
    }
}

fn spawn_resize_thread(
    session_name: String,
    should_stop: Arc<AtomicBool>,
) -> Result<(Handle, std::thread::JoinHandle<()>)> {
    let mut signals = Signals::new([signal_hook::consts::SIGWINCH])?;
    let handle = signals.handle();

    let join_handle = std::thread::spawn(move || {
        for _ in signals.forever() {
            if should_stop.load(Ordering::Relaxed) {
                return;
            }
            send_current_size(&session_name);
        }
    });

    Ok((handle, join_handle))
}

/// Signal used by the thread-exit channel to indicate which side finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitSide {
    Input,
    Output,
}

/// Attach to a running session via the daemon socket.
///
/// After connecting, the daemon forwards PTY output through a subscriber
/// channel to this socket. We:
/// 1. Send the current terminal size immediately so the attached PTY has the
///    correct dimensions before interaction starts
/// 2. Set the local terminal to raw mode
/// 3. Spawn threads to bridge stdin→socket and socket→stdout
/// 4. Watch for Ctrl-b then 'd' (detach sequence)
/// 5. Forward later terminal resize events via a separate side-channel request
///
/// **Shutdown coordination**: the main thread does NOT join input_thread first.
/// Instead it waits on a channel that either thread signals when it exits.
/// This ensures attach returns immediately when:
///   - user presses Ctrl-b d (input thread signals)
///   - daemon closes socket or PTY exits (output thread gets EOF)
///   - any I/O error occurs on either side
///
/// Without this channel-based approach, if the output thread exits (e.g. PTY
/// closed) while the input thread is blocked on `stdin.read()`, the main thread
/// would hang waiting for `input_thread.join()` — requiring the user to press
/// a key before attach returns. The channel avoids that entirely.
///
/// The stdin thread cannot be reliably unblocked from `read()` without
/// platform-specific tricks (SIGUSR1 injection or non-blocking stdin), so
/// after shutdown we detach it rather than blocking on join.
///
/// On detach (Ctrl-b d), only the client connection closes.
/// The session continues running in the daemon.
///
/// Important resize shutdown detail: `Signals::forever()` blocks waiting for a
/// signal. Without calling `Handle::close()` on detach, joining the resize
/// thread can hang forever if no SIGWINCH arrives. Closing the handle wakes the
/// iterator, lets the thread exit promptly, and makes detach return immediately.
pub fn attach_to_session(stream: UnixStream, session_name: &str) -> Result<()> {
    // Set the PTY size immediately so the first screen draw uses the correct
    // terminal dimensions even before the next SIGWINCH.
    send_current_size(session_name);

    // Set raw mode.
    let _guard = TerminalGuard::new()?;

    let should_stop = Arc::new(AtomicBool::new(false));

    let stream_read = stream.try_clone()?;

    // --- Resize forwarding via side-channel request ---
    let (resize_handle, resize_thread) =
        spawn_resize_thread(session_name.to_string(), should_stop.clone())?;

    // Channel: either thread signals when it exits.
    let (exit_tx, exit_rx) = mpsc::channel::<ExitSide>();

    // --- Socket → stdout (PTY output from subscriber) ---
    let should_stop_out = should_stop.clone();
    let exit_tx_out = exit_tx.clone();
    let output_thread = std::thread::spawn(move || {
        let mut stream = stream_read;
        let mut stdout = io::stdout();
        let mut buf = [0u8; 4096];
        loop {
            if should_stop_out.load(Ordering::Relaxed) {
                break;
            }
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stdout.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = stdout.flush();
                }
                Err(_) => break,
            }
        }
        let _ = exit_tx_out.send(ExitSide::Output);
    });

    // --- stdin → socket (with Ctrl-b d detection) ---
    let should_stop_in = should_stop.clone();
    let stream_write_for_input = stream.try_clone()?;
    let exit_tx_in = exit_tx.clone();
    let input_thread = std::thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut writer = stream_write_for_input;
        let mut buf = [0u8; 1024];
        let mut ctrl_b_pressed = false;

        loop {
            if should_stop_in.load(Ordering::Relaxed) {
                break;
            }
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // Check for detach sequence: Ctrl-b (0x02) followed by 'd'
                    if n == 1 && buf[0] == 0x02 {
                        ctrl_b_pressed = true;
                        continue;
                    }
                    if n == 1 && ctrl_b_pressed && buf[0] == b'd' {
                        should_stop_in.store(true, Ordering::SeqCst);
                        break;
                    }
                    ctrl_b_pressed = false;

                    // Forward input to PTY via socket.
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                Err(_) => break,
            }
        }
        let _ = exit_tx_in.send(ExitSide::Input);
    });

    // Drop our copy of exit_tx so the channel closes when both threads drop theirs.
    drop(exit_tx);

    // Wait for either thread to signal. This is the critical fix:
    // we do NOT join input_thread first — we wait on the channel.
    // This means: if output thread exits (PTY closed, daemon dropped connection),
    // we wake immediately without needing stdin to unblock.
    let _ = exit_rx.recv();

    // One side exited — initiate full shutdown.
    should_stop.store(true, Ordering::SeqCst);

    // Shutdown the socket to unblock both threads.
    let _ = stream.shutdown(std::net::Shutdown::Both);

    // Close the signal iterator so the resize thread wakes up and exits even if
    // no SIGWINCH arrives after detach.
    resize_handle.close();

    // Best-effort joins. The output thread should exit promptly (socket shut down).
    // The input thread may still be blocked on stdin.read() — we can't safely
    // interrupt a blocking read on stdin without platform-specific tricks, so
    // we detach it rather than hang. The thread will exit when the user presses
    // a key or when the process exits.
    let _ = output_thread.join();
    let _ = resize_thread.join();
    // Input thread is intentionally NOT joined: it may be blocked in stdin.read().
    // Detaching it avoids hanging attach_to_session if stdin hasn't received input.
    // The thread will terminate naturally when the process exits or stdin gets data.
    drop(input_thread);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn resize_thread_shutdown_does_not_block() {
        let should_stop = Arc::new(AtomicBool::new(false));
        let (handle, join_handle) =
            spawn_resize_thread("test-session".to_string(), should_stop).unwrap();

        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = join_handle.join();
            let _ = done_tx.send(());
        });

        handle.close();

        assert!(done_rx.recv_timeout(Duration::from_secs(1)).is_ok());
    }

    /// Test that the exit channel fires when the output side closes.
    /// This simulates the daemon closing the socket while stdin is still open.
    #[test]
    fn exit_channel_fires_on_output_close() {
        let (exit_tx, exit_rx) = mpsc::channel::<ExitSide>();
        let exit_tx_clone = exit_tx.clone();
        drop(exit_tx);

        // Simulate output thread exiting.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            let _ = exit_tx_clone.send(ExitSide::Output);
        });

        // Main thread should wake from recv() promptly.
        let result = exit_rx.recv_timeout(Duration::from_secs(1));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ExitSide::Output);
    }

    /// Test that the exit channel fires when the input side closes (detach).
    #[test]
    fn exit_channel_fires_on_input_detach() {
        let (exit_tx, exit_rx) = mpsc::channel::<ExitSide>();
        let exit_tx_clone = exit_tx.clone();
        drop(exit_tx);

        // Simulate input thread exiting (Ctrl-b d).
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            let _ = exit_tx_clone.send(ExitSide::Input);
        });

        let result = exit_rx.recv_timeout(Duration::from_secs(1));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ExitSide::Input);
    }

    /// Test that the exit channel detects disconnect (both senders dropped).
    #[test]
    fn exit_channel_detects_disconnect() {
        let (exit_tx, exit_rx) = mpsc::channel::<ExitSide>();
        drop(exit_tx);

        let result = exit_rx.recv_timeout(Duration::from_secs(1));
        assert!(result.is_err());
    }
}
