//! Integration test for workspace flow with isolated state.
//!
//! Tests the daemon protocol and workspace helpers end-to-end
//! using an isolated AGENTMUX_DATA_DIR.

use std::sync::OnceLock;
use tempfile::TempDir;

static TEST_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

fn get_lock() -> &'static std::sync::Mutex<()> {
    TEST_LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// Set up an isolated data directory and return the TempDir.
/// The caller must hold the TempDir for the test duration.
fn setup_isolated_state() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_DATA_DIR", dir.path());
    dir
}

/// Write a minimal .agentmux.toml in a temp project dir.
fn write_project_config(project_dir: &std::path::Path) {
    let config = r#"[workspace]
name = "integration-test"

[agents.shell]
command = "bash"
args = []

[[workspace.sessions]]
name = "worker-1"
profile = "shell"
cwd = "."

[[workspace.sessions]]
name = "worker-2"
profile = "shell"
cwd = "."
"#;
    std::fs::write(project_dir.join(".agentmux.toml"), config).unwrap();
}

#[test]
fn workspace_config_loads_correctly() {
    let _lock = get_lock().lock().unwrap();
    let _data_dir = setup_isolated_state();
    let project_dir = tempfile::tempdir().unwrap();
    write_project_config(project_dir.path());

    // Change to project dir and load config.
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(project_dir.path()).unwrap();

    let config = agentmux::config::Config::load().unwrap();

    // Verify workspace config was loaded.
    assert!(config.agents.contains_key("shell"));
    assert_eq!(config.agents.get("shell").unwrap().command, "bash");

    // Restore cwd.
    std::env::set_current_dir(orig).unwrap();
}

#[test]
fn metadata_persists_across_save_load_cycles() {
    let _lock = get_lock().lock().unwrap();
    let _data_dir = setup_isolated_state();

    use agentmux::daemon::metadata::{load_metadata, SessionMetadata};

    // Write metadata directly.
    let meta = vec![
        SessionMetadata {
            id: "id-1".to_string(),
            name: "session-a".to_string(),
            profile: "shell".to_string(),
            command: "bash".to_string(),
            args: vec![],
            cwd: None,
            pid: None,
            status: "exited".to_string(),
            exit_code: Some(0),
            created_at: 100,
            updated_at: 200,
            log_path: None,
        },
        SessionMetadata {
            id: "id-2".to_string(),
            name: "session-b".to_string(),
            profile: "shell".to_string(),
            command: "vim".to_string(),
            args: vec!["-e".to_string()],
            cwd: Some("/tmp".to_string()),
            pid: Some(42),
            status: "orphaned".to_string(),
            exit_code: None,
            created_at: 300,
            updated_at: 400,
            log_path: Some("/tmp/b.log".to_string()),
        },
    ];

    let path = agentmux::daemon::metadata::sessions_file_path().unwrap();
    std::fs::write(&path, serde_json::to_string_pretty(&meta).unwrap()).unwrap();

    let loaded = load_metadata().unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].name, "session-a");
    assert_eq!(loaded[0].status, "exited");
    assert_eq!(loaded[0].exit_code, Some(0));
    assert_eq!(loaded[1].name, "session-b");
    assert_eq!(loaded[1].status, "orphaned");
    assert_eq!(loaded[1].pid, Some(42));
    assert_eq!(loaded[1].command, "vim");
}

#[test]
fn recover_sessions_classifies_pid_status() {
    let _lock = get_lock().lock().unwrap();
    let _data_dir = setup_isolated_state();

    use agentmux::daemon::metadata::{recover_sessions, SessionMetadata};

    // Mix of live PID (our own process) and dead PID.
    let my_pid = std::process::id();
    let meta = vec![
        SessionMetadata {
            id: "live".to_string(),
            name: "live-s".to_string(),
            profile: "shell".to_string(),
            command: "bash".to_string(),
            args: vec![],
            cwd: None,
            pid: Some(my_pid),
            status: "running".to_string(),
            exit_code: None,
            created_at: 1,
            updated_at: 1,
            log_path: None,
        },
        SessionMetadata {
            id: "dead".to_string(),
            name: "dead-s".to_string(),
            profile: "shell".to_string(),
            command: "bash".to_string(),
            args: vec![],
            cwd: None,
            pid: Some(999999),
            status: "running".to_string(),
            exit_code: None,
            created_at: 1,
            updated_at: 1,
            log_path: None,
        },
    ];

    let path = agentmux::daemon::metadata::sessions_file_path().unwrap();
    std::fs::write(&path, serde_json::to_string_pretty(&meta).unwrap()).unwrap();

    let recovered = recover_sessions().unwrap();
    assert_eq!(recovered.len(), 2);

    let live = recovered.iter().find(|m| m.name == "live-s").unwrap();
    assert_eq!(live.status, "orphaned");

    let dead = recovered.iter().find(|m| m.name == "dead-s").unwrap();
    assert_eq!(dead.status, "exited");
}

#[test]
fn shutdown_protocol_request_serializes() {
    let request = agentmux::daemon::protocol::Request::Shutdown;
    let json = serde_json::to_string(&request).unwrap();
    let deserialized: agentmux::daemon::protocol::Request = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        deserialized,
        agentmux::daemon::protocol::Request::Shutdown
    ));
}

#[test]
fn daemon_status_protocol_request_serializes() {
    let request = agentmux::daemon::protocol::Request::DaemonStatus;
    let json = serde_json::to_string(&request).unwrap();
    let deserialized: agentmux::daemon::protocol::Request = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        deserialized,
        agentmux::daemon::protocol::Request::DaemonStatus
    ));
}

#[test]
fn orphaned_status_serializes_correctly() {
    use agentmux::daemon::session::SessionStatus;

    let status = SessionStatus::Orphaned;
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(json, "\"orphaned\"");

    let deserialized: SessionStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, SessionStatus::Orphaned);
    assert_eq!(status.to_string(), "orphaned");
}

#[test]
fn cli_parsing_remains_backward_compatible() {
    use agentmux::cli::Cli;
    use clap::Parser;

    // All these should parse without error.
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("daemon", vec!["agentmux", "daemon"]),
        ("daemon shutdown", vec!["agentmux", "daemon", "shutdown"]),
        ("daemon status", vec!["agentmux", "daemon", "status"]),
        ("run", vec!["agentmux", "run", "shell"]),
        (
            "run with name",
            vec!["agentmux", "run", "shell", "--name", "x"],
        ),
        ("list", vec!["agentmux", "list"]),
        ("list all", vec!["agentmux", "list", "--all"]),
        ("list verbose", vec!["agentmux", "list", "--verbose"]),
        ("attach", vec!["agentmux", "attach", "s1"]),
        ("stop", vec!["agentmux", "stop", "s1"]),
        ("kill", vec!["agentmux", "kill", "s1"]),
        ("logs", vec!["agentmux", "logs", "s1"]),
        ("send", vec!["agentmux", "send", "s1", "hi"]),
        ("status", vec!["agentmux", "status"]),
        ("profiles", vec!["agentmux", "profiles"]),
        ("config path", vec!["agentmux", "config", "path"]),
        ("workspace start", vec!["agentmux", "workspace", "start"]),
        ("tui", vec!["agentmux", "tui"]),
        ("doctor", vec!["agentmux", "doctor"]),
        ("restart", vec!["agentmux", "restart", "s1"]),
        ("version", vec!["agentmux", "version"]),
        ("reset stale", vec!["agentmux", "reset", "--stale"]),
        ("reset all", vec!["agentmux", "reset", "--all"]),
    ];

    for (name, args) in cases {
        let cli = Cli::try_parse_from(args);
        assert!(cli.is_ok(), "Failed to parse '{}': {:?}", name, cli.err());
    }
}
