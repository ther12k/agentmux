use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum WorkspaceAction {
    /// Start all configured workspace sessions
    Start,
    /// Show status of workspace sessions
    Status,
    /// Restart failed/exited/missing sessions
    RestartFailed,
}

pub fn handle_workspace_action(action: &WorkspaceAction) -> Result<()> {
    crate::daemon::autostart::ensure_daemon_running()?;
    match action {
        WorkspaceAction::Start => crate::workspace::start_workspace(),
        WorkspaceAction::Status => crate::workspace::workspace_status(),
        WorkspaceAction::RestartFailed => crate::workspace::restart_failed(),
    }
}
