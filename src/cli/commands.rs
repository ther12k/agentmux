use anyhow::Result;

use crate::config::Config;
use crate::daemon::autostart;
use crate::daemon::protocol::Request;
use crate::daemon::server;

pub use super::output::version_string;

/// Handle `agentmux run <profile>`
pub fn handle_run(profile: &str, name: Option<&str>, cwd: Option<&str>) -> Result<()> {
    let config = Config::load()?;
    let resolved = config.resolve_profile(profile)?;

    // Check command exists
    if which::which(&resolved.command).is_err() {
        anyhow::bail!(
            "Command '{}' not found in PATH (profile: {})",
            resolved.command,
            profile
        );
    }

    autostart::ensure_daemon_running()?;

    let resolved_name = name.unwrap_or(profile);

    let request = Request::SpawnSession {
        name: resolved_name.to_string(),
        profile: profile.to_string(),
        command: resolved.command.clone(),
        args: resolved.args.clone(),
        cwd: cwd.map(|s| s.to_string()),
    };

    let resp = server::send_request(&request)?;
    if !resp.ok {
        anyhow::bail!(
            "Failed to spawn session '{}': {:?}",
            resolved_name,
            resp.data
        );
    }
    let pid = resp.data.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("Session spawned: {} (pid={})", resolved_name, pid);
    Ok(())
}

/// Handle `agentmux list [--verbose] [--all]`
pub fn handle_list(verbose: bool, all: bool) -> Result<()> {
    autostart::ensure_daemon_running()?;

    let response = server::send_request(&Request::ListSessions)?;

    if !verbose {
        println!("{:<15} {:<15} {:<8} STATUS", "NAME", "COMMAND", "PID");
    }

    if let Some(sessions) = response.data.as_array() {
        let filtered: Vec<&serde_json::Value> = if all {
            sessions.iter().collect()
        } else {
            sessions
                .iter()
                .filter(|s| {
                    let status = s.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    !matches!(status, "exited" | "orphaned")
                })
                .collect()
        };

        if filtered.is_empty() {
            println!("No sessions");
        } else {
            for s in &filtered {
                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("-");
                let command = s.get("command").and_then(|v| v.as_str()).unwrap_or("-");
                let pid = s
                    .get("pid")
                    .and_then(|v| v.as_u64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let status = s.get("status").and_then(|v| v.as_str()).unwrap_or("-");

                if verbose {
                    let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("-");
                    let profile = s.get("profile").and_then(|v| v.as_str()).unwrap_or("-");
                    let args = s
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            if a.is_empty() {
                                "(none)".to_string()
                            } else {
                                a.iter()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            }
                        })
                        .unwrap_or_else(|| "(none)".to_string());
                    let cwd = s.get("cwd").and_then(|v| v.as_str()).unwrap_or("(none)");
                    let created_at = s
                        .get("created_at")
                        .and_then(|v| v.as_u64())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let updated_at = s
                        .get("updated_at")
                        .and_then(|v| v.as_u64())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let exit_code = s
                        .get("exit_code")
                        .and_then(|v| v.as_i64())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "(none)".to_string());
                    let log_path = s
                        .get("log_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(none)");

                    println!("Session: {}", name);
                    println!("  id:         {}", id);
                    println!("  name:       {}", name);
                    println!("  profile:    {}", profile);
                    println!("  command:    {}", command);
                    println!("  args:       {}", args);
                    println!("  cwd:        {}", cwd);
                    println!("  pid:        {}", pid);
                    println!("  status:     {}", status);
                    println!("  created_at: {}", created_at);
                    println!("  updated_at: {}", updated_at);
                    println!("  exit_code:  {}", exit_code);
                    println!("  log_path:   {}", log_path);
                    println!("---");
                } else {
                    println!("{:<15} {:<15} {:<8} {}", name, command, pid, status);
                }
            }
        }
    }
    Ok(())
}

/// Handle `agentmux attach <session>`
pub fn handle_attach(session: &str) -> Result<()> {
    autostart::ensure_daemon_running()?;

    let stream = server::connect_attach(session)?;
    crate::pty::attach::attach_to_session(stream, session)?;
    println!("\n[detached from {}]", session);
    Ok(())
}

/// Handle `agentmux stop <session>`
pub fn handle_stop(session: &str) -> Result<()> {
    autostart::ensure_daemon_running()?;

    let request = Request::StopSession {
        name: session.to_string(),
    };
    let resp = server::send_request(&request)?;
    if resp.ok {
        println!("Stopped: {}", session);
    } else {
        anyhow::bail!("Failed to stop '{}': {:?}", session, resp.data);
    }
    Ok(())
}

/// Handle `agentmux kill <session>`
pub fn handle_kill(session: &str) -> Result<()> {
    autostart::ensure_daemon_running()?;

    let request = Request::KillSession {
        name: session.to_string(),
    };
    let resp = server::send_request(&request)?;
    if resp.ok {
        println!("Killed: {}", session);
    } else {
        anyhow::bail!("Failed to kill '{}': {:?}", session, resp.data);
    }
    Ok(())
}

/// Handle `agentmux logs <session> --tail N`
pub fn handle_logs(session: &str, tail: usize) -> Result<()> {
    let _config = Config::load()?;
    let log_path = crate::storage::logs::log_file_path(session);

    match crate::storage::logs::tail_log(&log_path, tail) {
        Ok(lines) => {
            if lines.is_empty() {
                println!("No logs for session '{}'", session);
            } else {
                for line in lines {
                    println!("{}", line);
                }
            }
        }
        Err(e) => {
            if e.to_string().contains("not found") {
                println!("No log file found for session '{}'", session);
            } else {
                anyhow::bail!("Failed to read logs: {}", e);
            }
        }
    }
    Ok(())
}

/// Handle `agentmux send <session> <text> [--enter]`
pub fn handle_send(session: &str, text: &str, enter: bool) -> Result<()> {
    autostart::ensure_daemon_running()?;

    let mut data = text.to_string().into_bytes();
    if enter {
        data.push(b'\n');
    }

    let request = Request::SendInput {
        name: session.to_string(),
        data,
    };
    let resp = server::send_request(&request)?;
    if resp.ok {
        println!("Sent to {}: {:?}", session, text);
    } else {
        anyhow::bail!("Failed to send to '{}': {:?}", session, resp.data);
    }
    Ok(())
}

/// Handle `agentmux profiles`
pub fn handle_profiles() -> Result<()> {
    let config = Config::load()?;
    println!("{:<15} {:<20} ARGS", "PROFILE", "COMMAND");
    for name in config.agent_names() {
        let Some(p) = config.agents.get(name) else {
            continue;
        };
        let args = if p.args.is_empty() {
            "(none)".to_string()
        } else {
            p.args.join(" ")
        };
        println!("{:<15} {:<20} {}", name, p.command, args);
    }
    Ok(())
}

/// Handle `agentmux restart <session>`
pub fn handle_restart(session: &str) -> Result<()> {
    autostart::ensure_daemon_running()?;

    let request = Request::RestartSession {
        name: session.to_string(),
    };
    let resp = server::send_request(&request)?;
    if resp.ok {
        let pid = resp.data.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("Restarted: {} (pid={})", session, pid);
    } else {
        let err = resp
            .data
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("Failed to restart '{}': {}", session, err);
    }
    Ok(())
}
