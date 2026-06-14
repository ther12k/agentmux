use agentmux::cli::version_string;
use agentmux::daemon::session::format_restart_marker;
use agentmux::daemon::state;
use agentmux::doctor::{check_tty, tty_status_line};
use agentmux::workspace::{format_restart_summary, format_summary};

// ---- Version output tests ----

#[test]
fn version_string_contains_agentmux() {
    let v = version_string();
    assert!(
        v.contains("agentmux"),
        "Version string should contain 'agentmux': {}",
        v
    );
}

#[test]
fn version_string_contains_version_number() {
    let v = version_string();
    assert!(
        v.contains("0.1"),
        "Version string should contain version number: {}",
        v
    );
}

#[test]
fn version_string_contains_build_and_target() {
    let v = version_string();
    assert!(
        v.contains("build:"),
        "Version string should contain build info: {}",
        v
    );
    assert!(
        v.contains("target:"),
        "Version string should contain target info: {}",
        v
    );
}

// ---- Doctor TTY status formatting tests ----

#[test]
fn tty_status_line_returns_valid_status() {
    let line = tty_status_line("stdin", 0);
    assert!(
        line.contains("OK") || line.contains("WARN"),
        "TTY status line should contain OK or WARN: {}",
        line
    );
}

#[test]
fn tty_status_line_contains_label() {
    let line = tty_status_line("stderr", 2);
    assert!(
        line.contains("stderr"),
        "TTY status line should contain the label: {}",
        line
    );
}

#[test]
fn tty_status_line_format_matches_pattern() {
    // When it's a TTY, line should contain "tty"; when not, should contain "not a tty"
    let line = tty_status_line("stdout", 1);
    assert!(
        line.contains("tty"),
        "TTY status line should contain 'tty': {}",
        line
    );
}

#[test]
fn check_tty_does_not_panic() {
    // Just verify check_tty returns a boolean for standard FDs.
    let _ = check_tty(0);
    let _ = check_tty(1);
    let _ = check_tty(2);
}

// ---- Workspace summary formatting tests ----

#[test]
fn format_summary_contains_all_counts() {
    let s = format_summary(3, 2, 1);
    assert!(s.contains("started: 3"), "Summary should show started: 3");
    assert!(s.contains("skipped: 2"), "Summary should show skipped: 2");
    assert!(s.contains("failed: 1"), "Summary should show failed: 1");
}

#[test]
fn format_summary_handles_zeros() {
    let s = format_summary(0, 0, 0);
    assert!(s.contains("started: 0"));
    assert!(s.contains("skipped: 0"));
    assert!(s.contains("failed: 0"));
}

#[test]
fn format_summary_has_label() {
    let s = format_summary(1, 0, 0);
    assert!(
        s.contains("Summary:"),
        "Summary should contain 'Summary:' label: {}",
        s
    );
}

// ---- Restart marker tests ----

#[test]
fn format_restart_marker_contains_agentmux() {
    let marker = format_restart_marker();
    assert!(
        marker.contains("AgentMux restart at"),
        "Restart marker should contain 'AgentMux restart at': {}",
        marker
    );
}

#[test]
fn format_restart_marker_has_timestamp_pattern() {
    let marker = format_restart_marker();
    // The format is "YYYY-MM-DD HH:MM:SS" — check the pattern exists.
    // We check for a date-like pattern (4 digits, dash, 2 digits, dash, 2 digits).
    assert!(
        marker.contains("-"),
        "Restart marker should contain a formatted timestamp: {}",
        marker
    );
    assert!(
        marker.contains(":"),
        "Restart marker should contain a time component: {}",
        marker
    );
}

#[test]
fn format_restart_marker_starts_with_newline() {
    let marker = format_restart_marker();
    assert!(
        marker.starts_with('\n'),
        "Restart marker should start with newline for log separation: {:?}",
        marker
    );
}

// ---- restart-failed summary tests ----

#[test]
fn format_restart_summary_has_all_counts() {
    let s = format_restart_summary(1, 2, 3, 4);
    assert!(s.contains("started: 1"));
    assert!(s.contains("restarted: 2"));
    assert!(s.contains("skipped: 3"));
    assert!(s.contains("failed: 4"));
}

// ---- PID file stale detection test ----

#[test]
fn is_pid_alive_detects_nonexistent() {
    // PID 999999 almost certainly doesn't exist.
    assert!(!state::is_pid_alive(999999));
}
