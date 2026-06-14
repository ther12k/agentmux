use anyhow::Result;

use crate::config::Config;
use crate::daemon::protocol::Request;
use crate::daemon::server;
use crate::daemon::state;

/// Check if the given file descriptor is a TTY.
pub fn check_tty(fd: i32) -> bool {
    unsafe { libc::isatty(fd) != 0 }
}

/// Format a TTY status line for doctor output.
///
/// Returns e.g. `"stdin        OK   tty"` or `"stdin        WARN not a tty"`.
pub fn tty_status_line(label: &str, fd: i32) -> String {
    if check_tty(fd) {
        format!("{:<13} OK   tty", label)
    } else {
        format!("{:<13} WARN not a tty", label)
    }
}

/// Run the `agentmux doctor` diagnostics command.
/// Checks daemon, socket, config, profiles, terminal, and environment.
pub fn run_doctor() -> Result<()> {
    println!("AgentMux Doctor\n");

    let mut warnings = 0;
    let mut errors = 0;

    // --- Daemon section ---
    println!("Daemon:");

    // Binary path.
    let exe = std::env::current_exe().unwrap_or_else(|_| {
        errors += 1;
        std::path::PathBuf::from("(unknown)")
    });
    println!("  binary       {}", exe.display());

    // Daemon status.
    if state::is_daemon_running() {
        let pid_str = match server::send_request(&Request::Ping) {
            Ok(resp) if resp.ok => "running".to_string(),
            _ => {
                warnings += 1;
                "socket exists but unresponsive".to_string()
            }
        };
        println!("  status       OK   {}", pid_str);
    } else {
        warnings += 1;
        println!("  status       WARN not running");
    }

    // Socket path.
    match state::socket_path() {
        Ok(sock) => {
            print!("  socket       ");
            if sock.exists() {
                println!("OK   {}", sock.display());
            } else {
                warnings += 1;
                println!("WARN {} (not found)", sock.display());
            }
        }
        Err(_) => {
            errors += 1;
            println!("  socket       FAILED cannot determine path");
        }
    }

    // Data directory.
    match state::socket_dir() {
        Ok(dir) => {
            print!("  data_dir     ");
            if dir.exists() {
                println!("OK   {}", dir.display());
            } else {
                warnings += 1;
                println!("WARN {} (not found)", dir.display());
            }
        }
        Err(_) => {
            errors += 1;
            println!("  data_dir     FAILED cannot determine");
        }
    }

    // Log directory.
    match state::socket_dir() {
        Ok(dir) => {
            let log_dir = dir.join("logs");
            print!("  log_dir      ");
            if log_dir.exists() {
                println!("OK   {}", log_dir.display());
            } else {
                println!("WARN {} (will be created on first use)", log_dir.display());
            }
        }
        Err(_) => {
            println!("  log_dir      N/A");
        }
    }

    // Daemon PID (best-effort, read from PID file).
    match state::read_pid_file() {
        Some(pid) if state::is_pid_alive(pid) => {
            println!("  daemon_pid   OK   {}", pid);
        }
        Some(pid) => {
            warnings += 1;
            println!(
                "  daemon_pid   WARN stale pid file: {} (process not running)",
                pid
            );
            // Best-effort cleanup of stale PID file.
            state::remove_pid_file();
        }
        None => {
            println!("  daemon_pid   N/A  (no pid file)");
        }
    }

    // --- Config files ---
    println!("\nConfig:");

    match crate::config::global_config_path() {
        Some(p) => {
            print!("  global       ");
            if p.exists() {
                println!("OK   {}", p.display());
            } else {
                println!("N/A  {} (using defaults)", p.display());
            }
        }
        None => {
            println!("  global       N/A  (unable to determine path)");
        }
    }

    let proj_config = crate::config::project_config_path();
    print!("  project      ");
    if proj_config.exists() {
        println!("OK   {}", proj_config.display());
    } else {
        println!("N/A  {} (no project config)", proj_config.display());
    }

    // --- Profiles ---
    println!("\nProfiles:");

    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            errors += 1;
            println!("  (failed to load config: {})", e);
            return finish_doctor(warnings, errors);
        }
    };

    let names = config.agent_names();
    for name in &names {
        let profile = match config.resolve_profile(name) {
            Ok(p) => p,
            Err(_) => {
                errors += 1;
                println!("  {:<13} FAILED profile not resolvable", name);
                continue;
            }
        };
        match which::which(&profile.command) {
            Ok(path) => {
                println!(
                    "  {:<13} OK   {} ({})",
                    name,
                    profile.command,
                    path.display()
                );
            }
            Err(_) => {
                warnings += 1;
                println!(
                    "  {:<13} MISSING {} (command not found)",
                    name, profile.command
                );
            }
        }
    }

    // --- Interactive terminal ---
    println!("\nInteractive terminal:");

    let stdin_ok = check_tty(0);
    let stdout_ok = check_tty(1);
    let stderr_ok = check_tty(2);

    println!("  {}", tty_status_line("stdin", 0));
    println!("  {}", tty_status_line("stdout", 1));
    println!("  {}", tty_status_line("stderr", 2));

    if stdin_ok && stdout_ok && stderr_ok {
        println!("  attach/tui   OK   supported");
    } else {
        // One warning for the overall "attach/tui unavailable" status.
        warnings += 1;
        println!("  attach/tui   WARN unavailable in this environment");
    }

    // --- Environment ---
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "(not set)".to_string());
    println!("  shell        {}", shell);

    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unknown)".to_string());
    println!("  cwd          {}", cwd);

    // --- Version ---
    println!("\n  version      {}", env!("CARGO_PKG_VERSION"));

    finish_doctor(warnings, errors)
}

fn finish_doctor(warnings: u32, errors: u32) -> Result<()> {
    println!();
    if errors > 0 {
        println!("Result: FAILED ({} errors, {} warnings)", errors, warnings);
    } else if warnings > 0 {
        println!("Result: OK with warnings ({} warnings)", warnings);
    } else {
        println!("Result: OK");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tty_status_line_format_ok() {
        // We can't control whether FDs are TTYs in the test environment,
        // but we can verify the format of either branch.
        let line = tty_status_line("stdin", 0);
        assert!(
            line.contains("OK   tty") || line.contains("WARN not a tty"),
            "Expected OK or WARN status, got: {}",
            line
        );
    }

    #[test]
    fn test_tty_status_line_contains_label() {
        let line = tty_status_line("stdout", 1);
        assert!(line.contains("stdout"));
    }

    #[test]
    fn test_check_tty_returns_bool() {
        // Just verify it doesn't panic.
        let _ = check_tty(0);
        let _ = check_tty(1);
        let _ = check_tty(2);
    }
}
