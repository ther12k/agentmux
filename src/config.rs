use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::profiles::{builtin_profiles, AgentProfile};

/// Workspace session definition from project config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSession {
    pub name: String,
    pub profile: String,
    #[serde(default)]
    pub cwd: Option<String>,
}

/// Workspace section from project config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Workspace {
    pub name: Option<String>,
    #[serde(default)]
    pub sessions: Vec<WorkspaceSession>,
}

/// Top-level agentmux configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Agent profiles keyed by name.
    #[serde(default)]
    pub agents: HashMap<String, AgentProfile>,

    /// Workspace configuration (project-level only).
    #[serde(default)]
    pub workspace: Workspace,
}

impl Config {
    /// Merge built-in defaults first, then global, then project config.
    pub fn load() -> Result<Self> {
        let mut config = Self::builtin();

        // Load global config
        if let Some(global_path) = global_config_path() {
            if global_path.exists() {
                let contents = std::fs::read_to_string(&global_path)?;
                let parsed: Config = toml::from_str(&contents)?;
                config.merge(parsed);
            }
        }

        // Load project config (overrides global)
        let project_path = project_config_path();
        if project_path.exists() {
            let contents = std::fs::read_to_string(&project_path)?;
            let parsed: Config = toml::from_str(&contents)?;
            config.merge(parsed);
        }

        Ok(config)
    }

    /// Start with built-in default profiles.
    pub fn builtin() -> Self {
        let mut agents = HashMap::new();
        for (name, profile) in builtin_profiles() {
            agents.insert(name.to_string(), profile);
        }
        Config {
            agents,
            workspace: Workspace::default(),
        }
    }

    /// Merge another config into this one. Other takes priority.
    pub fn merge(&mut self, other: Config) {
        for (name, profile) in other.agents {
            self.agents.insert(name, profile);
        }
        // Only adopt workspace from project-level config.
        if !other.workspace.sessions.is_empty() || other.workspace.name.is_some() {
            self.workspace = other.workspace;
        }
    }

    /// Resolve a profile name to an AgentProfile.
    pub fn resolve_profile(&self, name: &str) -> Result<&AgentProfile> {
        self.agents.get(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown profile: '{}'. Available: {}",
                name,
                self.agent_names().join(", ")
            )
        })
    }

    /// Get sorted list of agent profile names.
    pub fn agent_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.agents.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    /// Validate the configuration. Returns a Vec of warning/error messages.
    /// Empty Vec = valid with no warnings.
    pub fn validate(&self) -> Result<Vec<String>> {
        let mut warnings = Vec::new();

        // 1. TOML already parsed by load() — OK.

        // 2. Workspace sessions have unique names.
        let mut seen_names = std::collections::HashSet::new();
        for session in &self.workspace.sessions {
            if !seen_names.insert(&session.name) {
                warnings.push(format!(
                    "duplicate workspace session name: '{}'",
                    session.name
                ));
            }
        }

        // 3. Every workspace session references an existing profile.
        for session in &self.workspace.sessions {
            if !self.agents.contains_key(&session.profile) {
                warnings.push(format!(
                    "session '{}' references unknown profile '{}'",
                    session.name, session.profile
                ));
            }
        }

        // 4. Every profile has a non-empty command.
        for (name, profile) in &self.agents {
            if profile.command.is_empty() {
                warnings.push(format!("profile '{}' has an empty command", name));
            }
        }

        // 5. Relative cwd is valid from project root (must be absolute or start with '.').
        for session in &self.workspace.sessions {
            if let Some(cwd) = &session.cwd {
                if !cwd.starts_with('.')
                    && !cwd.starts_with('/')
                    && !std::path::Path::new(cwd).is_absolute()
                {
                    warnings.push(format!(
                        "session '{}' has invalid cwd '{}' (must be absolute or relative starting with '.')",
                        session.name, cwd
                    ));
                }
            }
        }

        // 6. Command exists in PATH (warning only).
        for (name, profile) in &self.agents {
            if !profile.command.is_empty() && which::which(&profile.command).is_err() {
                warnings.push(format!(
                    "profile '{}' command '{}' not found in PATH",
                    name, profile.command
                ));
            }
        }

        Ok(warnings)
    }
}

/// Get the global config file path: ~/.config/agentmux/config.toml
pub fn global_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "agentmux", "agentmux")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}

/// Get the project-local config path: ./.agentmux.toml
pub fn project_config_path() -> PathBuf {
    PathBuf::from(".agentmux.toml")
}
