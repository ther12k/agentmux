use agentmux::config::Config;
use agentmux::profiles::AgentProfile;

#[test]
fn builtin_config_has_all_default_profiles() {
    let config = Config::builtin();
    let names = config.agent_names();
    assert!(names.contains(&"pi"));
    assert!(names.contains(&"codex"));
    assert!(names.contains(&"gemini"));
    assert!(names.contains(&"glm"));
    assert!(names.contains(&"aider"));
    assert!(names.contains(&"opencode"));
    assert!(names.contains(&"shell"));
}

#[test]
fn shell_profile_uses_bash() {
    let config = Config::builtin();
    let shell = config.resolve_profile("shell").unwrap();
    assert_eq!(shell.command, "bash");
    assert!(shell.args.is_empty());
}

#[test]
fn resolve_unknown_profile_returns_error() {
    let config = Config::builtin();
    let err = config.resolve_profile("nonexistent").unwrap_err();
    assert!(err.to_string().contains("Unknown profile"));
    assert!(err.to_string().contains("nonexistent"));
}

#[test]
fn merge_overrides_existing_profile() {
    let mut config = Config::builtin();
    let mut override_config = Config::default();
    override_config.agents.insert(
        "shell".to_string(),
        AgentProfile {
            command: "zsh".to_string(),
            args: vec!["-l".to_string()],
        },
    );
    config.merge(override_config);

    let shell = config.resolve_profile("shell").unwrap();
    assert_eq!(shell.command, "zsh");
    assert_eq!(shell.args, vec!["-l"]);
}

#[test]
fn merge_adds_new_profile() {
    let mut config = Config::builtin();
    let mut override_config = Config::default();
    override_config.agents.insert(
        "custom".to_string(),
        AgentProfile {
            command: "my-tool".to_string(),
            args: vec!["--flag".to_string()],
        },
    );
    config.merge(override_config);

    let custom = config.resolve_profile("custom").unwrap();
    assert_eq!(custom.command, "my-tool");
}

#[test]
fn agent_names_are_sorted() {
    let config = Config::builtin();
    let names = config.agent_names();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn example_workspace_config_parses() {
    let toml_str = r#"
[workspace]
name = "agentmux-dev"

[agents.pi]
command = "pi"
args = []

[agents.codex]
command = "codex"
args = []

[agents.shell]
command = "bash"
args = []

[[workspace.sessions]]
name = "pi-main"
profile = "pi"
cwd = "."

[[workspace.sessions]]
name = "shell"
profile = "shell"
cwd = "."
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.workspace.name.as_deref(), Some("agentmux-dev"));
    assert_eq!(config.workspace.sessions.len(), 2);
    assert_eq!(config.workspace.sessions[0].name, "pi-main");
    assert_eq!(config.workspace.sessions[0].profile, "pi");
    assert_eq!(config.workspace.sessions[1].name, "shell");

    // Profiles are present.
    assert!(config.agents.contains_key("pi"));
    assert!(config.agents.contains_key("codex"));
    assert!(config.agents.contains_key("shell"));
}

#[test]
fn workspace_sessions_default_to_empty() {
    let config = Config::default();
    assert!(config.workspace.sessions.is_empty());
    assert!(config.workspace.name.is_none());
}

#[test]
fn validate_catches_duplicate_session_names() {
    let toml_str = r#"
[agents.shell]
command = "bash"
args = []

[[workspace.sessions]]
name = "dup"
profile = "shell"
cwd = "."

[[workspace.sessions]]
name = "dup"
profile = "shell"
cwd = "."
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let warnings = config.validate().unwrap();
    let dup_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("duplicate"))
        .collect();
    assert!(!dup_warnings.is_empty(), "Expected duplicate name warning");
}

#[test]
fn validate_catches_missing_profile_reference() {
    let toml_str = r#"
[agents.shell]
command = "bash"
args = []

[[workspace.sessions]]
name = "s1"
profile = "nonexistent"
cwd = "."
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let warnings = config.validate().unwrap();
    let profile_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("unknown profile"))
        .collect();
    assert!(
        !profile_warnings.is_empty(),
        "Expected unknown profile warning"
    );
}

#[test]
fn validate_warns_for_missing_command() {
    let toml_str = r#"
[agents.broken]
command = ""
args = []
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let warnings = config.validate().unwrap();
    let cmd_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("empty command"))
        .collect();
    assert!(!cmd_warnings.is_empty(), "Expected empty command warning");
}

#[test]
fn validate_passes_for_good_config() {
    let toml_str = r#"
[agents.shell]
command = "bash"
args = []

[[workspace.sessions]]
name = "s1"
profile = "shell"
cwd = "."
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    let warnings = config.validate().unwrap();
    // The only potential warning is "bash not found in PATH" — but bash exists.
    // If bash is missing in the test environment, we still accept as long as
    // there are no structural errors.
    let structural: Vec<_> = warnings
        .iter()
        .filter(|w| {
            w.contains("duplicate")
                || w.contains("unknown profile")
                || w.contains("empty command")
                || w.contains("invalid cwd")
        })
        .collect();
    assert!(
        structural.is_empty(),
        "Expected no structural warnings, got: {:?}",
        structural
    );
}
