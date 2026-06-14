use serde::{Deserialize, Serialize};

/// A single agent profile definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// The command to execute (e.g. "pi", "codex", "bash").
    pub command: String,

    /// Extra arguments to pass.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Return default built-in profiles.
pub fn builtin_profiles() -> Vec<(&'static str, AgentProfile)> {
    vec![
        (
            "pi",
            AgentProfile {
                command: "pi".to_string(),
                args: vec![],
            },
        ),
        (
            "codex",
            AgentProfile {
                command: "codex".to_string(),
                args: vec![],
            },
        ),
        (
            "gemini",
            AgentProfile {
                command: "gemini".to_string(),
                args: vec![],
            },
        ),
        (
            "glm",
            AgentProfile {
                command: "glm".to_string(),
                args: vec![],
            },
        ),
        (
            "aider",
            AgentProfile {
                command: "aider".to_string(),
                args: vec![],
            },
        ),
        (
            "opencode",
            AgentProfile {
                command: "opencode".to_string(),
                args: vec![],
            },
        ),
        (
            "shell",
            AgentProfile {
                command: "bash".to_string(),
                args: vec![],
            },
        ),
    ]
}
