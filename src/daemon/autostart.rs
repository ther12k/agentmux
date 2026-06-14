use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::{debug, info, warn};

use super::state;

/// Maximum time to wait for auto-started daemon to become responsive.
const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(5);

/// Check if the daemon is running. If not, start it in the background.
///
/// This is called before any command that needs the daemon:
/// run, list, attach, stop, kill, logs, send, workspace start.
///
/// Returns Ok(()) if the daemon is running (either already or newly started).
/// Returns Err only if the daemon fails to start within the timeout.
pub fn ensure_daemon_running() -> Result<()> {
    if state::is_daemon_running() {
        return Ok(());
    }

    info!("Daemon not running — auto-starting");

    // Determine the binary path: use std::env::current_exe() so we re-invoke
    // the same binary (works whether installed or run from target/).
    let exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("Cannot determine agentmux binary path: {}", e))?;

    // Spawn `agentmux daemon` as a detached background process.
    let child = Command::new(&exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to start daemon: {}", e))?;

    let pid = child.id();
    debug!("Spawned daemon process (pid={})", pid);

    // Detach: we don't wait for it.
    drop(child);

    // Wait for the socket to become connectable.
    let deadline = Instant::now() + DAEMON_START_TIMEOUT;
    while Instant::now() < deadline {
        if state::is_daemon_running() {
            // Socket exists and is connectable. Give it a moment to finish
            // binding the listener.
            std::thread::sleep(Duration::from_millis(100));

            // Verify with a Ping.
            if let Ok(resp) = super::server::send_request(&super::protocol::Request::Ping) {
                if resp.ok {
                    info!("Daemon auto-started successfully (pid={})", pid);
                    return Ok(());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    warn!(
        "Daemon did not become responsive within {:?}",
        DAEMON_START_TIMEOUT
    );

    anyhow::bail!(
        "Daemon failed to start within {:?}. Try running 'agentmux daemon' manually.",
        DAEMON_START_TIMEOUT
    )
}
