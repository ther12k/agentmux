pub mod commands;
pub mod config;
pub mod daemon;
pub mod output;
pub mod reset;
pub mod workspace;

// Re-export version_string for backward compat (tests use it).
pub use output::version_string;

use anyhow::Result;
use clap::{Parser, Subcommand};

use self::config::ConfigAction;
use self::daemon::DaemonAction;
use self::workspace::WorkspaceAction;

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
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },

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

        /// Include exited/orphaned sessions
        #[arg(long)]
        all: bool,
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

impl Cli {
    pub fn run(&self) -> Result<()> {
        match &self.command {
            None => {
                println!("agentmux — run 'agentmux --help' for usage");
                Ok(())
            }
            Some(cmd) => match cmd {
                Command::Daemon { action } => {
                    match action {
                        None => {
                            // Default: start the daemon (backward compat)
                            tracing::info!("Starting agentmux daemon");
                            crate::daemon::server::run()?;
                            Ok(())
                        }
                        Some(DaemonAction::Shutdown) => crate::cli::daemon::handle_shutdown(),
                        Some(DaemonAction::Status) => crate::cli::daemon::handle_daemon_status(),
                    }
                }
                Command::Run { profile, name, cwd } => {
                    crate::cli::commands::handle_run(profile, name.as_deref(), cwd.as_deref())
                }
                Command::List { verbose, all } => crate::cli::commands::handle_list(*verbose, *all),
                Command::Attach { session } => crate::cli::commands::handle_attach(session),
                Command::Stop { session } => crate::cli::commands::handle_stop(session),
                Command::Kill { session } => crate::cli::commands::handle_kill(session),
                Command::Logs { session, tail } => {
                    crate::cli::commands::handle_logs(session, *tail)
                }
                Command::Send {
                    session,
                    text,
                    enter,
                } => crate::cli::commands::handle_send(session, text, *enter),
                Command::Status => crate::cli::daemon::handle_status(),
                Command::Profiles => crate::cli::commands::handle_profiles(),
                Command::Config { action } => crate::cli::config::handle_config_action(action),
                Command::Workspace { action } => {
                    crate::cli::workspace::handle_workspace_action(action)
                }
                Command::Tui => crate::tui::run_tui(),
                Command::Doctor => crate::doctor::run_doctor(),
                Command::Restart { session } => crate::cli::commands::handle_restart(session),
                Command::Version => {
                    println!("{}", crate::cli::commands::version_string());
                    Ok(())
                }
                Command::Reset { stale, all } => crate::cli::reset::handle_reset(*stale, *all),
            },
        }
    }
}

// Re-export for backward compat (main.rs uses Cli)
pub use Cli as CliStruct;

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_daemon_no_subcommand() {
        let cli = Cli::parse_from(["agentmux", "daemon"]);
        assert!(matches!(
            cli.command,
            Some(Command::Daemon { action: None })
        ));
    }

    #[test]
    fn parse_daemon_shutdown() {
        let cli = Cli::parse_from(["agentmux", "daemon", "shutdown"]);
        assert!(matches!(
            cli.command,
            Some(Command::Daemon {
                action: Some(DaemonAction::Shutdown)
            })
        ));
    }

    #[test]
    fn parse_daemon_status() {
        let cli = Cli::parse_from(["agentmux", "daemon", "status"]);
        assert!(matches!(
            cli.command,
            Some(Command::Daemon {
                action: Some(DaemonAction::Status)
            })
        ));
    }

    #[test]
    fn parse_list_all_flag() {
        let cli = Cli::parse_from(["agentmux", "list", "--all"]);
        match cli.command {
            Some(Command::List { all, .. }) => assert!(all),
            _ => panic!("Expected List command"),
        }
    }

    #[test]
    fn parse_list_verbose() {
        let cli = Cli::parse_from(["agentmux", "list", "--verbose"]);
        match cli.command {
            Some(Command::List { verbose, .. }) => assert!(verbose),
            _ => panic!("Expected List command"),
        }
    }

    #[test]
    fn parse_run() {
        let cli = Cli::parse_from(["agentmux", "run", "shell", "--name", "test"]);
        match cli.command {
            Some(Command::Run { profile, name, cwd }) => {
                assert_eq!(profile, "shell");
                assert_eq!(name.as_deref(), Some("test"));
                assert!(cwd.is_none());
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn parse_attach() {
        let cli = Cli::parse_from(["agentmux", "attach", "my-session"]);
        match cli.command {
            Some(Command::Attach { session }) => assert_eq!(session, "my-session"),
            _ => panic!("Expected Attach command"),
        }
    }

    #[test]
    fn parse_stop() {
        let cli = Cli::parse_from(["agentmux", "stop", "s1"]);
        assert!(matches!(
            cli.command,
            Some(Command::Stop { session }) if session == "s1"
        ));
    }

    #[test]
    fn parse_kill() {
        let cli = Cli::parse_from(["agentmux", "kill", "s1"]);
        assert!(matches!(
            cli.command,
            Some(Command::Kill { session }) if session == "s1"
        ));
    }

    #[test]
    fn parse_logs() {
        let cli = Cli::parse_from(["agentmux", "logs", "s1", "--tail", "100"]);
        match cli.command {
            Some(Command::Logs { session, tail }) => {
                assert_eq!(session, "s1");
                assert_eq!(tail, 100);
            }
            _ => panic!("Expected Logs command"),
        }
    }

    #[test]
    fn parse_send() {
        let cli = Cli::parse_from(["agentmux", "send", "s1", "hello world", "--enter"]);
        match cli.command {
            Some(Command::Send {
                session,
                text,
                enter,
            }) => {
                assert_eq!(session, "s1");
                assert_eq!(text, "hello world");
                assert!(enter);
            }
            _ => panic!("Expected Send command"),
        }
    }

    #[test]
    fn parse_status() {
        let cli = Cli::parse_from(["agentmux", "status"]);
        assert!(matches!(cli.command, Some(Command::Status)));
    }

    #[test]
    fn parse_profiles() {
        let cli = Cli::parse_from(["agentmux", "profiles"]);
        assert!(matches!(cli.command, Some(Command::Profiles)));
    }

    #[test]
    fn parse_config_path() {
        let cli = Cli::parse_from(["agentmux", "config", "path"]);
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                action: ConfigAction::Path
            })
        ));
    }

    #[test]
    fn parse_workspace_start() {
        let cli = Cli::parse_from(["agentmux", "workspace", "start"]);
        assert!(matches!(
            cli.command,
            Some(Command::Workspace {
                action: WorkspaceAction::Start
            })
        ));
    }

    #[test]
    fn parse_tui() {
        let cli = Cli::parse_from(["agentmux", "tui"]);
        assert!(matches!(cli.command, Some(Command::Tui)));
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::parse_from(["agentmux", "doctor"]);
        assert!(matches!(cli.command, Some(Command::Doctor)));
    }

    #[test]
    fn parse_restart() {
        let cli = Cli::parse_from(["agentmux", "restart", "s1"]);
        assert!(matches!(
            cli.command,
            Some(Command::Restart { session }) if session == "s1"
        ));
    }

    #[test]
    fn parse_version() {
        let cli = Cli::parse_from(["agentmux", "version"]);
        assert!(matches!(cli.command, Some(Command::Version)));
    }

    #[test]
    fn parse_reset_stale() {
        let cli = Cli::parse_from(["agentmux", "reset", "--stale"]);
        assert!(matches!(
            cli.command,
            Some(Command::Reset {
                stale: true,
                all: false
            })
        ));
    }

    #[test]
    fn parse_reset_all() {
        let cli = Cli::parse_from(["agentmux", "reset", "--all"]);
        assert!(matches!(
            cli.command,
            Some(Command::Reset {
                stale: false,
                all: true
            })
        ));
    }

    #[test]
    fn parse_no_command() {
        let cli = Cli::parse_from(["agentmux"]);
        assert!(cli.command.is_none());
    }
}
