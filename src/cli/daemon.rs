use anyhow::Result;
use clap::Subcommand;

use crate::daemon::protocol::Request;

#[derive(Subcommand, Debug)]
pub enum DaemonAction {
    /// Gracefully shut down the daemon
    Shutdown,
    /// Show daemon status (pid, uptime, sessions)
    Status,
}

/// Handle `agentmux status` — basic daemon ping.
pub fn handle_status() -> Result<()> {
    if crate::daemon::state::is_daemon_running() {
        match crate::daemon::server::send_request(&Request::Ping) {
            Ok(resp) => {
                if resp.ok {
                    println!("Daemon: running");
                    let sock_path = crate::daemon::state::socket_path()?;
                    println!("Socket:  {}", sock_path.display());
                } else {
                    println!("Daemon: error — {}", resp.data);
                }
            }
            Err(e) => {
                println!("Daemon: unreachable ({})", e);
            }
        }
    } else {
        println!("Daemon: not running");
    }
    Ok(())
}

/// Handle `agentmux daemon shutdown`
pub fn handle_shutdown() -> Result<()> {
    crate::daemon::autostart::ensure_daemon_running()?;

    let resp = crate::daemon::server::send_request(&Request::Shutdown)?;
    if resp.ok {
        println!("Daemon shutting down...");
    } else {
        let err = resp
            .data
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("Failed to shut down daemon: {}", err);
    }
    Ok(())
}

/// Handle `agentmux daemon status`
pub fn handle_daemon_status() -> Result<()> {
    crate::daemon::autostart::ensure_daemon_running()?;

    let resp = crate::daemon::server::send_request(&Request::DaemonStatus)?;
    if resp.ok {
        let pid = resp.data.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
        let uptime = resp
            .data
            .get("uptime_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let session_count = resp
            .data
            .get("session_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let socket_path = resp
            .data
            .get("socket_path")
            .and_then(|v| v.as_str())
            .unwrap_or("-");

        println!("Daemon:   running");
        println!("PID:      {}", pid);
        println!("Uptime:   {}s", uptime);
        println!("Sessions: {}", session_count);
        println!("Socket:   {}", socket_path);
    } else {
        let err = resp
            .data
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("Failed to get daemon status: {}", err);
    }
    Ok(())
}
