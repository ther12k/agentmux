use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::daemon::autostart;
use crate::daemon::protocol::Request;
use crate::daemon::server;

/// AgentMux — Lightweight terminal session manager for AI coding agents
#[derive(Parser, Debug)]
#[command(name = "agentmux", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the agentmux daemon manually
    Daemon,

    /// Run an agent in a new session
    Run {
        /// Profile name or command to run
        profile: String,

        /// Name for this session
        #[arg(long)]
        name: Option<String>,

        /// Working directory
        #[arg(long)]
        cwd: Option<String>,
    },

    /// List active sessions
    List {
        /// Show verbose details (log paths, etc.)
        #[arg(long)]
        verbose: bool,
    },

    /// Attach to a running session
    Attach {
        /// Session name
        session: String,
    },

    /// Stop a running session (SIGTERM)
    Stop {
        /// Session name
        session: String,
    },

    /// Force kill a running session (SIGKILL)
    Kill {
        /// Session name
        session: String,
    },

    /// Show recent output from a session
    Logs {
        /// Session name
        session: String,

        /// Number of lines to show
        #[arg(long, default_value = "50")]
        tail: usize,
    },

    /// Send text to a running session
    Send {
        /// Session name
        session: String,

        /// Text to send
        text: String,

        /// Append a newline after the text
        #[arg(long)]
        enter: bool,
    },

    /// Show daemon status
    Status,

    /// List available agent profiles
    Profiles,

    /// Config management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Manage project workspace sessions
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },

    /// Launch the TUI session switcher
    Tui,

    /// Run diagnostics checks
    Doctor,

    /// Restart a session (stop and respawn with same metadata)
    Restart {
        /// Session name to restart
        session: String,
    },

    /// Show version information
    Version,

    /// Reset daemon state (stale socket, etc.)
    Reset {
        /// Remove stale socket file if daemon is unresponsive
        #[arg(long)]
        stale: bool,

        /// Remove ALL state (socket + data dir). Does NOT remove logs.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Show config file path
    Path,
    /// Print merged config
    Print,
    /// Validate config and show warnings
    Validate,
}

#[derive(Subcommand, Debug)]
pub enum WorkspaceAction {
    /// Start all configured workspace sessions
    Start,
    /// Show status of workspace sessions
    Status,
    /// Restart failed/exited/missing sessions
    RestartFailed,
}

impl Cli {
    pub fn run(&self) -> Result<()> {
        match &self.command {
            None => {
                println!("agentmux — run 'agentmux --help' for usage");
                Ok(())
            }
            Some(cmd) => match cmd {
                Command::Daemon => {
                    tracing::info!("Starting agentmux daemon");
                    crate::daemon::server::run()?;
                    Ok(())
                }
                Command::Run { profile, name, cwd } => {
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

                    // Auto-start daemon if needed.
                    autostart::ensure_daemon_running()?;

                    let resolved_name = name.as_deref().unwrap_or(profile);

                    let request = Request::SpawnSession {
                        name: resolved_name.to_string(),
                        profile: profile.to_string(),
                        command: resolved.command.clone(),
                        args: resolved.args.clone(),
                        cwd: cwd.clone(),
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
                Command::List { verbose } => {
                    autostart::ensure_daemon_running()?;

                    let response = server::send_request(&Request::ListSessions)?;

                    if !*verbose {
                        println!("{:<15} {:<15} {:<8} STATUS", "NAME", "COMMAND", "PID");
                    }

                    if let Some(sessions) = response.data.as_array() {
                        if sessions.is_empty() {
                            println!("No sessions");
                        } else {
                            for s in sessions {
                                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("-");
                                let command =
                                    s.get("command").and_then(|v| v.as_str()).unwrap_or("-");
                                let pid = s
                                    .get("pid")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "-".to_string());
                                let status =
                                    s.get("status").and_then(|v| v.as_str()).unwrap_or("-");

                                if *verbose {
                                    let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("-");
                                    let profile =
                                        s.get("profile").and_then(|v| v.as_str()).unwrap_or("-");
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
                                    let cwd =
                                        s.get("cwd").and_then(|v| v.as_str()).unwrap_or("(none)");
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
                Command::Attach { session } => {
                    autostart::ensure_daemon_running()?;

                    let stream = server::connect_attach(session)?;
                    crate::pty::attach::attach_to_session(stream, session)?;
                    println!("\n[detached from {}]", session);
                    Ok(())
                }
                Command::Stop { session } => {
                    autostart::ensure_daemon_running()?;

                    let request = Request::StopSession {
                        name: session.clone(),
                    };
                    let resp = server::send_request(&request)?;
                    if resp.ok {
                        println!("Stopped: {}", session);
                    } else {
                        anyhow::bail!("Failed to stop '{}': {:?}", session, resp.data);
                    }
                    Ok(())
                }
                Command::Kill { session } => {
                    autostart::ensure_daemon_running()?;

                    let request = Request::KillSession {
                        name: session.clone(),
                    };
                    let resp = server::send_request(&request)?;
                    if resp.ok {
                        println!("Killed: {}", session);
                    } else {
                        anyhow::bail!("Failed to kill '{}': {:?}", session, resp.data);
                    }
                    Ok(())
                }
                Command::Logs { session, tail } => {
                    let _config = Config::load()?;
                    let log_path = crate::storage::logs::log_file_path(session);

                    match crate::storage::logs::tail_log(&log_path, *tail) {
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
                Command::Send {
                    session,
                    text,
                    enter,
                } => {
                    autostart::ensure_daemon_running()?;

                    let mut data = text.clone().into_bytes();
                    if *enter {
                        data.push(b'\n');
                    }

                    let request = Request::SendInput {
                        name: session.clone(),
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
                Command::Status => {
                    if crate::daemon::state::is_daemon_running() {
                        match server::send_request(&Request::Ping) {
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
                Command::Profiles => {
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
                Command::Config { action } => match action {
                    ConfigAction::Path => {
                        match crate::config::global_config_path() {
                            Some(p) => println!("Global config:  {}", p.display()),
                            None => println!("Global config:  (unable to determine)"),
                        }
                        println!(
                            "Project config: {}",
                            crate::config::project_config_path().display()
                        );
                        Ok(())
                    }
                    ConfigAction::Print => {
                        let config = Config::load()?;
                        let toml_str = toml::to_string_pretty(&config)?;
                        println!("{}", toml_str);
                        Ok(())
                    }
                    ConfigAction::Validate => {
                        let config = Config::load()?;
                        let warnings = config.validate()?;
                        if warnings.is_empty() {
                            println!("Config: OK");
                        } else {
                            println!("Config: OK with warnings:");
                            for w in &warnings {
                                println!("  WARN: {}", w);
                            }
                        }
                        Ok(())
                    }
                },
                Command::Workspace { action } => match action {
                    WorkspaceAction::Start => {
                        autostart::ensure_daemon_running()?;
                        crate::workspace::start_workspace()
                    }
                    WorkspaceAction::Status => {
                        autostart::ensure_daemon_running()?;
                        crate::workspace::workspace_status()
                    }
                    WorkspaceAction::RestartFailed => {
                        autostart::ensure_daemon_running()?;
                        crate::workspace::restart_failed()
                    }
                },
                Command::Tui => crate::tui::run_tui(),
                Command::Doctor => crate::doctor::run_doctor(),
                Command::Restart { session } => {
                    autostart::ensure_daemon_running()?;

                    let request = Request::RestartSession {
                        name: session.clone(),
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
                Command::Version => {
                    println!("{}", version_string());
                    Ok(())
                }
                Command::Reset { stale, all } => {
                    handle_reset(*stale, *all)?;
                    Ok(())
                }
            },
        }
    }
}

/// Format the version output string.
///
/// ```text
/// agentmux 0.1.0-alpha
/// build: release
/// target: x86_64-unknown-linux-gnu
/// ```
pub fn version_string() -> String {
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    // Construct a reasonable target triple from std constants.
    // std::env::consts::OS returns "linux" not "linux-gnu", so we add the ABI suffix.
    let abi = match std::env::consts::OS {
        "linux" => "linux-gnu",
        other => other,
    };
    let target = format!("{}-unknown-{}", std::env::consts::ARCH, abi);
    format!(
        "agentmux {}\nbuild: {}\ntarget: {}",
        env!("CARGO_PKG_VERSION"),
        build,
        target
    )
}

/// Handle the `agentmux reset` command.
///
/// - `--stale`: Remove stale socket file if daemon is unresponsive.
/// - `--all`: Same as --stale, plus remove data directory contents except logs.
///
/// Does NOT kill processes or delete logs.
fn handle_reset(stale: bool, all: bool) -> Result<()> {
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
