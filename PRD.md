# AgentMux PRD

## 1. Product Summary

AgentMux is a lightweight Rust-based terminal session manager for running multiple AI coding agents in parallel.

The first version focuses only on reliable terminal spawning, attaching, detaching, and managing multiple agent sessions. It should be small, fast, and practical for Linux developers who use agents such as Pi Agent, Codex, Gemini CLI, GLM, Aider, OpenCode, or other terminal-based tools.

AgentMux is not a full cmux clone at the start. It starts as a terminal/session core, then evolves into a cmux-like Linux AI-agent workspace.

## 2. Primary User

Software developer running multiple coding agents on Linux.

The user wants to:

* Start several agent terminals inside one project.
* Keep agents running after detaching.
* Attach back to any running agent.
* See which agents are active, idle, exited, or waiting.
* Avoid heavy Electron/Tauri resource usage.
* Use a small native Rust tool.

## 3. Main Problem

Running multiple AI agents manually across different terminal windows is messy.

Problems:

* Hard to track which agent is doing what.
* Easy to lose terminal state.
* No unified project session.
* Multiple windows consume focus.
* Agent tasks are difficult to resume.
* No simple way to spawn predefined agent profiles.

## 4. Product Goals

### v0.1 Goal

Create a Rust CLI/daemon that can spawn and manage multiple interactive PTY sessions.

The user should be able to run:

```bash
agentmux run pi --name pi-main --cwd .
agentmux run codex --name codex-ui --cwd .
agentmux list
agentmux attach pi-main
agentmux detach
agentmux stop codex-ui
```

### Long-Term Goal

Create a small native Linux cmux-like workspace for AI coding agents.

Future UI may include:

* Vertical agent tabs
* Split panes
* Workspace sidebar
* Agent status detection
* Notifications
* Localhost preview
* Project config
* Session restore

## 5. Non-Goals for v0.1

Do not build these in the first phase:

* Full GUI
* Browser preview
* Webview
* Plugin system
* Cloud sync
* Complex agent intelligence
* Full terminal renderer
* AI API integration
* Chat UI
* Project memory
* Git automation

v0.1 should focus only on terminal spawning and session management.

## 6. Target Platform

Initial target:

```txt
Linux x86_64
```

Preferred environment:

```txt
Ubuntu / Debian / Arch / WSL optional later
```

## 7. Technical Direction

AgentMux should be a native Rust application.

Recommended architecture:

```txt
agentmux CLI
    ↓
agentmux daemon
    ↓
PTY session manager
    ↓
agent processes
```

The daemon owns the running PTY sessions. The CLI communicates with the daemon using a Unix domain socket.

## 8. Core Components

### 8.1 CLI

Binary name:

```bash
agentmux
```

Commands:

```bash
agentmux daemon
agentmux run <profile-or-command>
agentmux list
agentmux attach <session>
agentmux stop <session>
agentmux kill <session>
agentmux logs <session>
agentmux send <session> <text>
agentmux status
```

### 8.2 Daemon

The daemon manages:

* Active sessions
* Session metadata
* PTY handles
* Child process lifecycle
* Session logs
* Socket API
* Attach/detach behavior

Daemon socket path:

```txt
~/.local/share/agentmux/agentmux.sock
```

### 8.3 Session

A session represents one running terminal process.

Session fields:

```txt
id
name
profile
command
args
cwd
pid
created_at
updated_at
status
exit_code
log_path
```

Session statuses:

```txt
running
attached
detached
exited
failed
```

### 8.4 PTY

Each session should run inside a real pseudo-terminal.

PTY requirements:

* Spawn shell or command.
* Support interactive input.
* Stream output.
* Resize terminal.
* Preserve process state while detached.
* Support attach/re-attach.

### 8.5 Agent Profiles

Agent profiles allow easy spawning of known tools.

Example config:

```toml
[agents.pi]
command = "pi"
args = []

[agents.codex]
command = "codex"
args = []

[agents.gemini]
command = "gemini"
args = []

[agents.glm]
command = "glm"
args = []

[agents.aider]
command = "aider"
args = []
```

Local project config:

```txt
.agentmux.toml
```

Global config:

```txt
~/.config/agentmux/config.toml
```

Project config should override global config.

## 9. MVP User Stories

### Story 1: Start Pi Agent

As a developer, I want to start Pi Agent inside my current project folder.

Acceptance:

```bash
agentmux run pi --name pi-main --cwd .
```

Creates one running PTY session.

### Story 2: List Sessions

As a developer, I want to see all running agent sessions.

Acceptance:

```bash
agentmux list
```

Shows:

```txt
NAME       PROFILE   STATUS     PID     CWD
pi-main    pi        detached   1234    /home/user/project
codex-ui   codex     running    1240    /home/user/project
```

### Story 3: Attach Session

As a developer, I want to attach to a running agent session.

Acceptance:

```bash
agentmux attach pi-main
```

Connects my terminal to the PTY session.

### Story 4: Detach Session

As a developer, I want to detach without killing the agent.

Acceptance:

```txt
Ctrl-b then d
```

Detaches from the session and keeps it running.

### Story 5: Stop Session

As a developer, I want to stop an agent session.

Acceptance:

```bash
agentmux stop pi-main
```

Gracefully sends SIGTERM. If it does not exit, user can run:

```bash
agentmux kill pi-main
```

### Story 6: Logs

As a developer, I want basic logs for each session.

Acceptance:

```bash
agentmux logs pi-main
```

Prints recent session output.

## 10. Suggested Rust Dependencies

Initial:

```toml
anyhow = "1"
clap = { version = "4", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
uuid = { version = "1", features = ["v4", "serde"] }
portable-pty = "0.9"
directories = "5"
tracing = "0.1"
tracing-subscriber = "0.3"
nix = "0.29"
```

Optional later:

```toml
rusqlite = { version = "0.32", features = ["bundled"] }
notify-rust = "4"
ratatui = "0.29"
crossterm = "0.28"
```

## 11. Suggested Project Structure

```txt
agentmux/
  Cargo.toml
  src/
    main.rs
    cli.rs
    config.rs
    daemon/
      mod.rs
      server.rs
      protocol.rs
      state.rs
    pty/
      mod.rs
      session.rs
      attach.rs
      resize.rs
    storage/
      mod.rs
      logs.rs
    profiles.rs
    error.rs
```

## 12. v0.1 Acceptance Criteria

v0.1 is complete when:

* `agentmux run pi --name pi-main --cwd .` spawns a real interactive PTY.
* `agentmux list` shows active sessions.
* `agentmux attach pi-main` attaches to the running session.
* User can detach without killing the process.
* Session keeps running after detach.
* `agentmux stop <name>` stops a session.
* Basic log file is written per session.
* Invalid commands return clear errors.
* No GUI or webview dependency is included.

## 13. Design Principle

Keep the core small.

AgentMux should first be a reliable terminal process manager. UI, splits, tabs, browser preview, notifications, and agent intelligence should come later.
