use serde::{Deserialize, Serialize};

/// Request from CLI to daemon.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// Check daemon liveness.
    Ping,

    /// List all sessions in daemon registry.
    ListSessions,

    /// Request daemon shutdown.
    Shutdown,

    /// Create/register a session entry in daemon registry (no spawn).
    AddSession {
        name: String,
        profile: String,
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
    },

    /// Remove a session entry from daemon registry.
    RemoveSession { name: String },

    /// Spawn a real PTY-backed child in the daemon.
    SpawnSession {
        name: String,
        profile: String,
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
    },

    /// Send SIGTERM to a running session.
    StopSession { name: String },

    /// Send SIGKILL to a running session.
    KillSession { name: String },

    /// Attach to a session's PTY.
    /// The connection transitions to raw byte forwarding mode
    /// after the response is sent.
    AttachSession { name: String },

    /// Detach from a session (clean up attach state on daemon side).
    DetachSession { name: String },

    /// Resize a session's PTY.
    ResizeSession { name: String, rows: u16, cols: u16 },

    /// Send text input to a session's PTY writer.
    SendInput { name: String, data: Vec<u8> },

    /// Restart a session (stop and respawn with same metadata).
    RestartSession { name: String },
}

/// Response from daemon to CLI.
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub data: serde_json::Value,
}

impl Response {
    pub fn ok(data: serde_json::Value) -> Self {
        Response { ok: true, data }
    }

    pub fn error(msg: &str) -> Self {
        Response {
            ok: false,
            data: serde_json::json!({ "error": msg }),
        }
    }
}
