use agentmux::daemon::protocol::{Request, Response};
use agentmux::daemon::session::{Session, SessionRegistry, SessionStatus};

// ---- Protocol tests ----

#[test]
fn ping_request_serializes_with_type_tag() {
    let json = serde_json::to_string(&Request::Ping).unwrap();
    assert!(json.contains(r#""type":"Ping""#));
}

#[test]
fn list_sessions_request_roundtrip() {
    let json = serde_json::to_string(&Request::ListSessions).unwrap();
    let req: Request = serde_json::from_str(&json).unwrap();
    assert!(matches!(req, Request::ListSessions));
}

#[test]
fn spawn_session_request_roundtrip() {
    let original = Request::SpawnSession {
        name: "test1".to_string(),
        profile: "shell".to_string(),
        command: "bash".to_string(),
        args: vec!["-c".to_string(), "echo hi".to_string()],
        cwd: Some("/tmp".to_string()),
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    match parsed {
        Request::SpawnSession {
            name,
            profile,
            command,
            args,
            cwd,
        } => {
            assert_eq!(name, "test1");
            assert_eq!(profile, "shell");
            assert_eq!(command, "bash");
            assert_eq!(args, vec!["-c", "echo hi"]);
            assert_eq!(cwd.as_deref(), Some("/tmp"));
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn stop_session_request_roundtrip() {
    let original = Request::StopSession {
        name: "s1".to_string(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, Request::StopSession { .. }));
}

#[test]
fn kill_session_request_roundtrip() {
    let original = Request::KillSession {
        name: "s1".to_string(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, Request::KillSession { .. }));
}

#[test]
fn resize_session_request_roundtrip() {
    let original = Request::ResizeSession {
        name: "s1".to_string(),
        rows: 50,
        cols: 120,
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    match parsed {
        Request::ResizeSession { name, rows, cols } => {
            assert_eq!(name, "s1");
            assert_eq!(rows, 50);
            assert_eq!(cols, 120);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn send_input_request_roundtrip() {
    let original = Request::SendInput {
        name: "s1".to_string(),
        data: b"echo hello\n".to_vec(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    match parsed {
        Request::SendInput { name, data } => {
            assert_eq!(name, "s1");
            assert_eq!(data, b"echo hello\n");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn send_input_request_empty_data() {
    let original = Request::SendInput {
        name: "empty".to_string(),
        data: vec![],
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    match parsed {
        Request::SendInput { name, data } => {
            assert_eq!(name, "empty");
            assert!(data.is_empty());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn shutdown_request_serializes() {
    let json = serde_json::to_string(&Request::Shutdown).unwrap();
    assert!(json.contains(r#""type":"Shutdown""#));
}

#[test]
fn restart_session_request_roundtrip() {
    let original = Request::RestartSession {
        name: "test-session".to_string(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    match parsed {
        Request::RestartSession { name } => {
            assert_eq!(name, "test-session");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn response_ok_construction() {
    let resp = Response::ok(serde_json::json!("pong"));
    assert!(resp.ok);
    assert_eq!(resp.data, serde_json::json!("pong"));
}

#[test]
fn response_error_construction() {
    let resp = Response::error("something failed");
    assert!(!resp.ok);
    assert_eq!(
        resp.data.get("error").and_then(|v| v.as_str()),
        Some("something failed")
    );
}

// ---- Session registry tests ----

#[test]
fn registry_starts_empty() {
    let reg = SessionRegistry::new();
    assert!(reg.list().is_empty());
}

#[test]
fn registry_add_and_get() {
    let mut reg = SessionRegistry::new();
    let session = Session::new("s1", "shell", "bash", vec![], None).unwrap();
    reg.add(session).unwrap();
    assert!(reg.exists("s1"));
    assert_eq!(reg.get("s1").unwrap().command, "bash");
}

#[test]
fn registry_rejects_duplicate() {
    let mut reg = SessionRegistry::new();
    reg.add(Session::new("dup", "shell", "bash", vec![], None).unwrap())
        .unwrap();
    let err = reg
        .add(Session::new("dup", "shell", "bash", vec![], None).unwrap())
        .unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

#[test]
fn registry_remove_clears_session() {
    let mut reg = SessionRegistry::new();
    reg.add(Session::new("temp", "shell", "bash", vec![], None).unwrap())
        .unwrap();
    assert!(reg.exists("temp"));
    let removed = reg.remove("temp");
    assert!(removed.is_some());
    assert!(!reg.exists("temp"));
    assert!(reg.list().is_empty());
}

#[test]
fn registry_list_preserves_insertion_order() {
    let mut reg = SessionRegistry::new();
    reg.add(Session::new("c", "shell", "bash", vec![], None).unwrap())
        .unwrap();
    reg.add(Session::new("a", "shell", "bash", vec![], None).unwrap())
        .unwrap();
    reg.add(Session::new("b", "shell", "bash", vec![], None).unwrap())
        .unwrap();

    let list = reg.list();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].name, "c");
    assert_eq!(list[1].name, "a");
    assert_eq!(list[2].name, "b");
}

#[test]
fn registry_update_changes_status_and_exit_code() {
    let mut reg = SessionRegistry::new();
    reg.add(Session::new("s1", "shell", "bash", vec![], None).unwrap())
        .unwrap();

    reg.update("s1", Some(SessionStatus::Exited), Some(42), Some(12345));

    let session = reg.get("s1").unwrap();
    assert!(matches!(session.status, SessionStatus::Exited));
    assert_eq!(session.exit_code, Some(42));
    assert_eq!(session.pid, Some(12345));
}

#[test]
fn registry_update_nonexistent_is_noop() {
    let mut reg = SessionRegistry::new();
    reg.update("ghost", Some(SessionStatus::Exited), Some(0), None);
    // Should not panic
}

#[test]
fn session_new_sets_correct_defaults() {
    let session = Session::new(
        "test",
        "shell",
        "bash",
        vec!["-l".to_string()],
        Some("/tmp".to_string()),
    )
    .unwrap();
    assert_eq!(session.name, "test");
    assert_eq!(session.profile, "shell");
    assert_eq!(session.command, "bash");
    assert_eq!(session.args, vec!["-l"]);
    assert_eq!(session.cwd.as_deref(), Some("/tmp"));
    assert!(session.pid.is_none());
    assert!(session.exit_code.is_none());
    assert!(session.log_path.is_none());
    assert!(matches!(session.status, SessionStatus::Running));
    assert!(!session.id.is_empty());
    assert!(session.created_at > 0);
    assert_eq!(session.created_at, session.updated_at);
}

// ---- Spawn and reap integration tests ----

#[test]
fn spawn_quick_exit_process_and_reap() {
    let mut reg = SessionRegistry::new();
    reg.add(Session::new("quick", "shell", "true", vec![], None).unwrap())
        .unwrap();

    let pid = reg.spawn("quick", "true", vec![], None).unwrap();
    assert!(pid > 0);

    // Poll for reap (up to 5 seconds)
    for _ in 0..50 {
        reg.reap();
        if let Some(s) = reg.get("quick") {
            if matches!(s.status, SessionStatus::Exited) {
                // `true` exits with code 0
                assert_eq!(s.exit_code, Some(0));
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("Session was not reaped within 5 seconds");
}

#[test]
fn spawn_false_exits_with_nonzero() {
    let mut reg = SessionRegistry::new();
    reg.add(Session::new("failer", "shell", "false", vec![], None).unwrap())
        .unwrap();

    reg.spawn("failer", "false", vec![], None).unwrap();

    for _ in 0..50 {
        reg.reap();
        if let Some(s) = reg.get("failer") {
            if matches!(s.status, SessionStatus::Exited) {
                assert_eq!(s.exit_code, Some(1));
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("Session was not reaped within 5 seconds");
}

#[test]
fn spawn_nonexistent_session_errors() {
    let mut reg = SessionRegistry::new();
    let err = reg.spawn("ghost", "true", vec![], None).unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn signal_nonexistent_session_errors() {
    let mut reg = SessionRegistry::new();
    let err = reg.signal("ghost", true).unwrap_err();
    assert!(err.to_string().contains("no running child"));
}

#[test]
fn kill_terminates_running_process() {
    let mut reg = SessionRegistry::new();
    reg.add(Session::new("victim", "shell", "sleep", vec![], None).unwrap())
        .unwrap();
    reg.spawn("victim", "sleep", vec!["100".to_string()], None)
        .unwrap();

    // Give it a moment to start
    std::thread::sleep(std::time::Duration::from_millis(200));

    // SIGKILL
    reg.signal("victim", true).unwrap();

    // Wait for reap
    for _ in 0..50 {
        reg.reap();
        if let Some(s) = reg.get("victim") {
            if matches!(s.status, SessionStatus::Exited) {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("Session was not killed+reaped within 5 seconds");
}
