use anyhow::Result;

/// Handle the `agentmux reset` command.
///
/// - `--stale`: Remove stale socket file if daemon is unresponsive.
/// - `--all`: Same as --stale, plus remove data directory contents except logs.
///
/// Does NOT kill processes or delete logs.
pub fn handle_reset(stale: bool, all: bool) -> Result<()> {
    if !stale && !all {
        anyhow::bail!(
            "No reset flag specified. Use:\n  --stale  Remove stale socket if daemon is unresponsive\n  --all    Remove ALL state (socket + data dir, NOT logs)"
        );
    }

    let daemon_alive = crate::daemon::state::is_daemon_running();

    // Handle socket removal.
    match crate::daemon::state::remove_stale_socket()? {
        Some(path) => {
            println!("Removed stale socket: {}", path.display());
        }
        None => {
            if daemon_alive {
                println!("Daemon is running — socket left intact.");
            } else {
                println!("No stale socket found.");
            }
        }
    }

    // Handle --all: remove data directory contents except logs, socket, and PID file.
    // If daemon is live, skip socket and PID file to avoid breaking active connections.
    if all {
        let data_dir = crate::daemon::state::socket_dir()?;
        let sock_path = crate::daemon::state::socket_path().ok();
        let pid_path = crate::daemon::state::pid_file_path().ok();
        if data_dir.exists() {
            let entries = std::fs::read_dir(&data_dir)?;
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Always skip the logs directory.
                if name_str == "logs" {
                    continue;
                }
                // Skip socket and PID file if daemon is live.
                if daemon_alive {
                    if let Some(sp) = &sock_path {
                        if path == *sp {
                            println!("Skipped socket (daemon running): {}", path.display());
                            continue;
                        }
                    }
                    if let Some(pp) = &pid_path {
                        if path == *pp {
                            println!("Skipped PID file (daemon running): {}", path.display());
                            continue;
                        }
                    }
                }
                if path.is_dir() {
                    std::fs::remove_dir_all(&path)?;
                    println!("Removed directory: {}", path.display());
                } else {
                    std::fs::remove_file(&path)?;
                    println!("Removed file: {}", path.display());
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_no_flag_returns_error() {
        let result = handle_reset(false, false);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No reset flag specified"));
    }
}
