use anyhow::Result;

use crate::config::Config;
use crate::daemon::protocol::Request;
use crate::daemon::server;

/// Format a workspace summary line.
pub fn format_summary(started: usize, skipped: usize, failed: usize) -> String {
    format!(
        "\nSummary:\n  started: {}\n  skipped: {}\n  failed: {}",
        started, skipped, failed
    )
}

/// Start all sessions configured in the project's `.agentmux.toml` [workspace] section.
///
/// - Skips sessions that are already running.
/// - Prints a clean summary table.
/// - Relative cwd paths are resolved from the current directory.
pub fn start_workspace() -> Result<()> {
    let config = Config::load()?;

    let workspace = &config.workspace;

    if workspace.sessions.is_empty() {
        println!("No workspace sessions configured.");
        println!("Add a [[workspace.sessions]] section to .agentmux.toml");
        return Ok(());
    }

    // Fetch currently running sessions to check for duplicates.
    let existing_sessions = fetch_session_names()?;

    if let Some(name) = &workspace.name {
        println!("Workspace: {}\n", name);
    } else {
        println!("Starting workspace\n");
    }

    let mut started = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for session in &workspace.sessions {
        // Check if already running.
        if existing_sessions.contains(&session.name) {
            println!(
                "{:<8} {:<15} {:<10} already running",
                "SKIPPED", session.name, session.profile
            );
            skipped += 1;
            continue;
        }

        // Resolve profile.
        let profile = match config.resolve_profile(&session.profile) {
            Ok(p) => p,
            Err(e) => {
                println!(
                    "{:<8} {:<15} {:<10} {}",
                    "FAILED", session.name, session.profile, e
                );
                failed += 1;
                continue;
            }
        };

        // Check command exists.
        if which::which(&profile.command).is_err() {
            println!(
                "{:<8} {:<15} {:<10} command '{}' not found",
                "FAILED", session.name, session.profile, profile.command
            );
            failed += 1;
            continue;
        }

        // Resolve cwd relative to current directory.
        let cwd = session
            .cwd
            .as_ref()
            .map(|c| {
                let path = std::path::PathBuf::from(c);
                if path.is_absolute() {
                    c.clone()
                } else {
                    std::env::current_dir()
                        .unwrap_or_default()
                        .join(path)
                        .to_string_lossy()
                        .into_owned()
                }
            })
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
            });

        let request = Request::SpawnSession {
            name: session.name.clone(),
            profile: session.profile.clone(),
            command: profile.command.clone(),
            args: profile.args.clone(),
            cwd,
        };

        match server::send_request(&request) {
            Ok(resp) => {
                if resp.ok {
                    let pid = resp.data.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                    println!(
                        "{:<8} {:<15} {:<10} pid={}",
                        "STARTED", session.name, session.profile, pid
                    );
                    started += 1;
                } else {
                    let err = resp
                        .data
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    println!(
                        "{:<8} {:<15} {:<10} {}",
                        "FAILED", session.name, session.profile, err
                    );
                    failed += 1;
                }
            }
            Err(e) => {
                println!(
                    "{:<8} {:<15} {:<10} daemon error: {}",
                    "FAILED", session.name, session.profile, e
                );
                failed += 1;
            }
        }
    }

    println!("{}", format_summary(started, skipped, failed));

    Ok(())
}

/// Fetch the set of existing session names from the daemon.
fn fetch_session_names() -> Result<Vec<String>> {
    let resp = server::send_request(&Request::ListSessions)?;
    let mut names = Vec::new();
    if let Some(arr) = resp.data.as_array() {
        for s in arr {
            if let Some(name) = s.get("name").and_then(|v| v.as_str()) {
                names.push(name.to_string());
            }
        }
    }
    Ok(names)
}

/// Fetch live session data from the daemon as a JSON array.
fn fetch_live_sessions() -> Result<Vec<serde_json::Value>> {
    let resp = server::send_request(&Request::ListSessions)?;
    if let Some(arr) = resp.data.as_array() {
        Ok(arr.clone())
    } else {
        Ok(Vec::new())
    }
}

/// Show the status of all workspace sessions.
pub fn workspace_status() -> Result<()> {
    let config = Config::load()?;
    let workspace = &config.workspace;

    if workspace.sessions.is_empty() {
        println!("No workspace sessions configured.");
        println!("Add a [[workspace.sessions]] section to .agentmux.toml");
        return Ok(());
    }

    let live = fetch_live_sessions().unwrap_or_default();

    // Build a lookup map of live sessions by name.
    let mut live_map: std::collections::HashMap<&str, &serde_json::Value> =
        std::collections::HashMap::new();
    for s in &live {
        if let Some(name) = s.get("name").and_then(|v| v.as_str()) {
            live_map.insert(name, s);
        }
    }

    // Print header.
    if let Some(name) = &workspace.name {
        println!("Workspace: {}\n", name);
    } else {
        println!("Workspace:\n");
    }

    println!(
        "{:<15} {:<10} {:<10} {:<8} CWD",
        "SESSION", "PROFILE", "STATUS", "PID"
    );

    let mut running_count = 0;
    let mut failed_count = 0;
    let mut missing_count = 0;

    for session in &workspace.sessions {
        let pid_str;
        let status_str;

        if let Some(live_s) = live_map.get(session.name.as_str()) {
            let live_status = live_s
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let pid = live_s
                .get("pid")
                .and_then(|v| v.as_u64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());

            match live_status {
                "running" | "attached" | "detached" => {
                    status_str = "running";
                    pid_str = pid;
                    running_count += 1;
                }
                "exited" => {
                    status_str = "exited";
                    pid_str = pid;
                    failed_count += 1;
                }
                "failed" => {
                    status_str = "failed";
                    pid_str = pid;
                    failed_count += 1;
                }
                _ => {
                    status_str = live_status;
                    pid_str = pid;
                }
            }
        } else {
            status_str = "missing";
            pid_str = "-".to_string();
            missing_count += 1;
        }

        let cwd_display = session.cwd.as_deref().unwrap_or("(default)");

        println!(
            "{:<15} {:<10} {:<10} {:<8} {}",
            session.name, session.profile, status_str, pid_str, cwd_display
        );
    }

    println!(
        "\nSummary: {} running, {} unhealthy, {} missing",
        running_count, failed_count, missing_count
    );

    Ok(())
}

/// Restart all failed/exited/missing workspace sessions.
///
/// - running / attached / detached → SKIPPED
/// - exited / failed → RestartSession (uses existing session metadata)
/// - missing (not in daemon) → SpawnSession (fresh spawn from workspace config)
pub fn restart_failed() -> Result<()> {
    let config = Config::load()?;
    let workspace = &config.workspace;

    if workspace.sessions.is_empty() {
        println!("No workspace sessions configured.");
        return Ok(());
    }

    let live = fetch_live_sessions().unwrap_or_default();

    // Build a lookup map of live sessions by name.
    let mut live_map: std::collections::HashMap<&str, &serde_json::Value> =
        std::collections::HashMap::new();
    for s in &live {
        if let Some(name) = s.get("name").and_then(|v| v.as_str()) {
            live_map.insert(name, s);
        }
    }

    if let Some(name) = &workspace.name {
        println!("Workspace: {}\n", name);
    } else {
        println!("Restarting workspace\n");
    }

    let mut started = 0;
    let mut restarted = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for session in &workspace.sessions {
        // Determine current status.
        let (is_healthy, is_present) = if let Some(live_s) = live_map.get(session.name.as_str()) {
            let st = live_s
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            (
                matches!(st, "running" | "attached" | "detached"),
                true, // session exists in daemon
            )
        } else {
            (false, false) // session not in daemon at all
        };

        if is_healthy {
            println!(
                "{:<10} {:<15} {:<10} already healthy",
                "SKIPPED", session.name, session.profile
            );
            skipped += 1;
            continue;
        }

        // Resolve profile.
        let profile = match config.resolve_profile(&session.profile) {
            Ok(p) => p,
            Err(e) => {
                println!(
                    "{:<10} {:<15} {:<10} {}",
                    "FAILED", session.name, session.profile, e
                );
                failed += 1;
                continue;
            }
        };

        // Check command exists.
        if which::which(&profile.command).is_err() {
            println!(
                "{:<10} {:<15} {:<10} command '{}' not found",
                "FAILED", session.name, session.profile, profile.command
            );
            failed += 1;
            continue;
        }

        if is_present {
            // Session exists but is exited/failed → restart it.
            let request = Request::RestartSession {
                name: session.name.clone(),
            };

            match server::send_request(&request) {
                Ok(resp) => {
                    if resp.ok {
                        let pid = resp.data.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                        println!(
                            "{:<10} {:<15} {:<10} pid={}",
                            "RESTARTED", session.name, session.profile, pid
                        );
                        restarted += 1;
                    } else {
                        let err = resp
                            .data
                            .get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown error");
                        println!(
                            "{:<10} {:<15} {:<10} {}",
                            "FAILED", session.name, session.profile, err
                        );
                        failed += 1;
                    }
                }
                Err(e) => {
                    println!(
                        "{:<10} {:<15} {:<10} daemon error: {}",
                        "FAILED", session.name, session.profile, e
                    );
                    failed += 1;
                }
            }
        } else {
            // Session is missing from daemon → fresh spawn.
            let cwd = session
                .cwd
                .as_ref()
                .map(|c| {
                    let path = std::path::PathBuf::from(c);
                    if path.is_absolute() {
                        c.clone()
                    } else {
                        std::env::current_dir()
                            .unwrap_or_default()
                            .join(path)
                            .to_string_lossy()
                            .into_owned()
                    }
                })
                .or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().into_owned())
                });

            let request = Request::SpawnSession {
                name: session.name.clone(),
                profile: session.profile.clone(),
                command: profile.command.clone(),
                args: profile.args.clone(),
                cwd,
            };

            match server::send_request(&request) {
                Ok(resp) => {
                    if resp.ok {
                        let pid = resp.data.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                        println!(
                            "{:<10} {:<15} {:<10} pid={}",
                            "STARTED", session.name, session.profile, pid
                        );
                        started += 1;
                    } else {
                        let err = resp
                            .data
                            .get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown error");
                        println!(
                            "{:<10} {:<15} {:<10} {}",
                            "FAILED", session.name, session.profile, err
                        );
                        failed += 1;
                    }
                }
                Err(e) => {
                    println!(
                        "{:<10} {:<15} {:<10} daemon error: {}",
                        "FAILED", session.name, session.profile, e
                    );
                    failed += 1;
                }
            }
        }
    }

    println!(
        "{}",
        format_restart_summary(started, restarted, skipped, failed)
    );

    Ok(())
}

/// Format the restart-failed summary with started + restarted + skipped + failed.
pub fn format_restart_summary(
    started: usize,
    restarted: usize,
    skipped: usize,
    failed: usize,
) -> String {
    format!(
        "\nSummary:\n  started: {}\n  restarted: {}\n  skipped: {}\n  failed: {}",
        started, restarted, skipped, failed
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_session_names_returns_empty_when_no_daemon() {
        // When no daemon is running, fetch_session_names returns an error.
        // We can't easily test the daemon path in unit tests, but we can
        // verify the function doesn't panic.
        let result = fetch_session_names();
        let _ = result;
    }

    #[test]
    fn test_format_summary() {
        let s = format_summary(1, 2, 3);
        assert!(s.contains("started: 1"));
        assert!(s.contains("skipped: 2"));
        assert!(s.contains("failed: 3"));
    }

    #[test]
    fn test_format_summary_zero() {
        let s = format_summary(0, 0, 0);
        assert!(s.contains("started: 0"));
        assert!(s.contains("skipped: 0"));
        assert!(s.contains("failed: 0"));
    }

    #[test]
    fn test_format_restart_summary() {
        let s = format_restart_summary(1, 2, 3, 4);
        assert!(s.contains("started: 1"));
        assert!(s.contains("restarted: 2"));
        assert!(s.contains("skipped: 3"));
        assert!(s.contains("failed: 4"));
    }

    #[test]
    fn test_format_restart_summary_zeros() {
        let s = format_restart_summary(0, 0, 0, 0);
        assert!(s.contains("started: 0"));
        assert!(s.contains("restarted: 0"));
        assert!(s.contains("skipped: 0"));
        assert!(s.contains("failed: 0"));
    }
}
