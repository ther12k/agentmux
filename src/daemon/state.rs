use std::path::PathBuf;

use anyhow::Result;

/// Get the daemon data directory.
///
/// Honors `AGENTMUX_DATA_DIR` env override for testing isolation.
/// Default: ~/.local/share/agentmux/
pub fn socket_dir() -> Result<PathBuf> {
    // Allow test isolation via env override.
    if let Ok(custom) = std::env::var("AGENTMUX_DATA_DIR") {
        let dir = PathBuf::from(custom);
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }
        return Ok(dir);
    }

    let home = directories::ProjectDirs::from("dev", "agentmux", "agentmux")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("agentmux")
        });

    if !home.exists() {
        std::fs::create_dir_all(&home)?;
    }

    Ok(home)
}

/// Get the daemon socket path: <data_dir>/agentmux.sock
pub fn socket_path() -> Result<PathBuf> {
    Ok(socket_dir()?.join("agentmux.sock"))
}

/// Get the daemon PID file path: <data_dir>/agentmux.pid
pub fn pid_file_path() -> Result<PathBuf> {
    Ok(socket_dir()?.join("agentmux.pid"))
}

/// Write the current process PID to the PID file.
/// Best-effort: warns on failure, does not crash.
pub fn write_pid_file() {
    let pid = std::process::id();
    if let Ok(path) = pid_file_path() {
        if let Err(e) = std::fs::write(&path, pid.to_string()) {
            tracing::warn!("Failed to write PID file {}: {}", path.display(), e);
        }
    }
}

/// Remove the PID file. Best-effort: warns on failure.
pub fn remove_pid_file() {
    if let Ok(path) = pid_file_path() {
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!("Failed to remove PID file {}: {}", path.display(), e);
            }
        }
    }
}

/// Read the daemon PID from the PID file.
/// Returns None if the file doesn't exist or can't be parsed.
pub fn read_pid_file() -> Option<u32> {
    let path = pid_file_path().ok()?;
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    content.trim().parse().ok()
}

/// Check if a process with the given PID is alive.
///
/// Uses `kill(pid, 0)` to probe process existence:
/// - `rc == 0` → process exists and we can signal it → alive
/// - `errno == EPERM` → process exists but we lack permission → alive
/// - `errno == ESRCH` → no such process → not alive
pub fn is_pid_alive(pid: u32) -> bool {
    // Safety: kill(pid, 0) is a well-defined POSIX call that does not send a
    // signal when sig=0. It only checks process existence and permissions.
    let rc = unsafe { libc::kill(pid as i32, 0) };
    if rc == 0 {
        return true;
    }
    // Check errno: EPERM means the process exists but we can't signal it.
    let errno = unsafe { *libc::__errno_location() };
    errno == libc::EPERM
}

/// Check if the daemon is running by trying to connect to the socket.
pub fn is_daemon_running() -> bool {
    let path = match socket_path() {
        Ok(p) => p,
        Err(_) => return false,
    };

    if !path.exists() {
        return false;
    }

    std::os::unix::net::UnixStream::connect(&path).is_ok()
}

/// Remove the stale socket file if the daemon is not responsive.
///
/// Returns:
/// - `Ok(Some(path))` if the socket was removed.
/// - `Ok(None)` if no socket exists or the daemon is alive (nothing removed).
pub fn remove_stale_socket() -> Result<Option<PathBuf>> {
    let path = socket_path()?;
    if !path.exists() {
        return Ok(None);
    }
    if is_daemon_running() {
        return Ok(None); // daemon is alive, don't remove
    }
    std::fs::remove_file(&path)?;
    Ok(Some(path))
}
