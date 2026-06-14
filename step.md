# AgentMux Phase-by-Phase Implementation Prompts

## Phase 0 — Repo Setup

You are building AgentMux, a lightweight Rust terminal session manager for running multiple AI coding agents.

Goal of this phase:
Create the initial Rust project structure only.

Requirements:

* Use Rust.
* Create a CLI binary named `agentmux`.
* Use `clap` for commands.
* Do not build PTY logic yet.
* Do not build GUI.
* Keep the code small and clean.
* Add basic modules:

  * cli
  * config
  * daemon
  * pty
  * profiles
  * storage
  * error

Commands that should compile:

```bash
agentmux --help
agentmux daemon
agentmux run --help
agentmux list
agentmux attach --help
agentmux stop --help
agentmux status
```

Implementation notes:

* `daemon`, `run`, `list`, `attach`, `stop`, and `status` can return placeholder messages for now.
* Add `anyhow`, `clap`, `tokio`, `serde`, `serde_json`, `toml`, `uuid`, `directories`, `tracing`, and `tracing-subscriber`.
* Add README with project goal and v0.1 scope.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Run `cargo build`.

Summarize:

* Files created.
* Commands supported.
* Any risks or unfinished items.

Do not over-engineer.
Do not add GUI.
Do not add browser preview.
Do not add AI API integration.

````

## Phase 1 — Config and Agent Profiles

Goal of this phase:
Add config loading for global and project-level agent profiles.

Requirements:
- Support global config:
```txt
~/.config/agentmux/config.toml
````

* Support project config:

```txt
.agentmux.toml
```

* Project config overrides global config.
* Add default built-in profiles:

```txt
pi
codex
gemini
glm
aider
opencode
shell
```

Example config:

```toml
[agents.pi]
command = "pi"
args = []

[agents.codex]
command = "codex"
args = []

[agents.shell]
command = "bash"
args = []
```

CLI requirements:

```bash
agentmux profiles
agentmux config path
agentmux config print
```

Run command should resolve profile names:

```bash
agentmux run pi --name pi-main --cwd .
```

But it does not need to spawn yet.

Acceptance:

* Config loads without crashing if missing.
* Built-in profiles work.
* Project config overrides global config.
* Invalid profile returns useful error.
* `agentmux config print` shows merged config.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Run `cargo build`.

Summarize changed files and risks.

````

## Phase 2 — Single PTY Spawn, Foreground Mode

Goal of this phase:
Implement the first real PTY spawn.

Important:
This phase does not need daemon/session persistence yet.

Requirement:
Add foreground run mode:

```bash
agentmux run pi --name pi-main --cwd .
````

Behavior:

* Resolve `pi` profile from config.
* Spawn the command inside a real PTY.
* Attach current terminal input/output to the PTY.
* Use raw terminal mode.
* Restore terminal mode on exit, Ctrl-C, panic, or child exit.
* Forward terminal resize to the PTY.
* Return child exit code if possible.

Implementation hints:

* Use `portable-pty`.
* Use `nix` or suitable crate for terminal raw mode and signals.
* Keep this as simple and robust as possible.
* Add a clear terminal cleanup guard.

Acceptance:

* `agentmux run shell --name test` opens an interactive shell.
* User can run commands inside the shell.
* `exit` returns to the original terminal correctly.
* Terminal is not broken after exit.
* `agentmux run pi --name pi-main --cwd .` attempts to start Pi Agent.
* If command is missing, show clear error.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Manual test with:

```bash
agentmux run shell --name test
```

Summarize:

* What works.
* What still does not persist after exit.
* Known terminal limitations.

````

## Phase 3 — Daemon and Unix Socket

Goal of this phase:
Create the AgentMux daemon and local socket API.

Requirement:
The daemon should run at:

```txt
~/.local/share/agentmux/agentmux.sock
````

Commands:

```bash
agentmux daemon
agentmux status
```

Behavior:

* `agentmux daemon` starts a long-running process.
* It creates the socket directory if missing.
* It removes stale socket files safely.
* It accepts JSON requests over Unix domain socket.
* It supports at least:

  * ping
  * list_sessions
  * shutdown daemon, optional and guarded

Protocol:
Use JSON lines or length-prefixed JSON. Keep it simple.

Example request:

```json
{ "type": "ping" }
```

Example response:

```json
{ "ok": true, "data": "pong" }
```

Acceptance:

* Daemon starts.
* `agentmux status` returns daemon status.
* If daemon is not running, `agentmux status` says so clearly.
* Socket errors are handled cleanly.
* No PTY sessions required yet.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Run `cargo build`.

Summarize changed files and risks.

````

## Phase 4 — Persistent Session Registry

Goal of this phase:
The daemon should maintain an in-memory session registry.

Requirement:
Add session model:

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
````

Commands:

```bash
agentmux list
```

Behavior:

* `agentmux list` asks daemon for sessions.
* If no sessions exist, show a clean empty state.
* Session names must be unique.
* Add internal session states:

```txt
running
attached
detached
exited
failed
```

No PTY spawn yet inside daemon.

Acceptance:

* Daemon maintains session registry.
* `agentmux list` works through socket.
* Duplicate session names are rejected.
* Tests cover session registry add/list/remove/update behavior.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Run `cargo build`.

Summarize changed files and risks.

````

## Phase 5 — Daemon-Owned PTY Sessions

Goal of this phase:
Move PTY spawning into the daemon so sessions survive CLI exit.

Command:
```bash
agentmux run pi --name pi-main --cwd .
````

Behavior:

* CLI sends spawn request to daemon.
* Daemon resolves command/profile.
* Daemon opens PTY.
* Daemon spawns child process.
* Daemon stores session in registry.
* CLI returns after successful spawn.
* Session remains running.

Add:

```bash
agentmux list
agentmux stop <session>
agentmux kill <session>
```

Acceptance:

* Starting a session does not block the CLI.
* `agentmux list` shows running session.
* Session keeps running after CLI exits.
* `agentmux stop <session>` sends SIGTERM.
* `agentmux kill <session>` sends SIGKILL.
* Exited process updates session status.
* Missing command returns clear error.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Manual test:

```bash
agentmux daemon
agentmux run shell --name shell1 --cwd .
agentmux list
agentmux stop shell1
```

Summarize changed files and risks.

````

## Phase 6 — Attach and Detach

Goal of this phase:
Allow user to attach to daemon-owned PTY sessions.

Command:
```bash
agentmux attach pi-main
````

Behavior:

* Attach current terminal to existing PTY session.
* Stream PTY output to stdout.
* Stream stdin to PTY.
* Raw mode enabled while attached.
* Raw mode restored after detach.
* Terminal resize is forwarded.
* Detach key:

```txt
Ctrl-b then d
```

Important:

* Detach must not kill the child process.
* If the child exits, attach should end gracefully.
* Only one attached client per session for now.

Acceptance:

* Start shell session:

```bash
agentmux run shell --name shell1 --cwd .
```

* Attach:

```bash
agentmux attach shell1
```

* Run commands interactively.
* Detach with Ctrl-b then d.
* Reattach and see the shell is still alive.
* Exit shell and session status becomes exited.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Manual test attach/detach repeatedly.

Summarize changed files, limitations, and risks.

````

## Phase 7 — Session Logs

Goal of this phase:
Add basic per-session logs.

Behavior:
- Every PTY output stream should be appended to a log file.
- Log path:
```txt
~/.local/share/agentmux/logs/<session-name>.log
````

Command:

```bash
agentmux logs <session>
agentmux logs <session> --tail 100
```

Acceptance:

* Logs are written while session runs.
* `agentmux logs pi-main` prints logs.
* `--tail` limits output.
* Logs do not crash on binary/invalid UTF-8 output.
* Log file path is shown in `agentmux list --verbose`.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Manual test with shell output.

Summarize changed files and risks.

````

## Phase 8 — Project Workspace Command

Goal of this phase:
Make AgentMux easy to use per project.

Command:
```bash
agentmux workspace start
````

Project config:

```toml
[workspace]
name = "my-project"

[[workspace.sessions]]
name = "pi-main"
profile = "pi"
cwd = "."

[[workspace.sessions]]
name = "codex-ui"
profile = "codex"
cwd = "."

[[workspace.sessions]]
name = "shell"
profile = "shell"
cwd = "."
```

Behavior:

* Reads `.agentmux.toml`.
* Starts all configured sessions.
* Skips sessions already running.
* Prints summary.

Acceptance:

* One command can start multiple agents.
* Duplicate running sessions are skipped safely.
* Missing profiles show clear errors.
* Relative cwd resolves from project root.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Manual test with sample `.agentmux.toml`.

Summarize changed files and risks.

````

## Phase 9 — Lightweight TUI Session Switcher

Goal of this phase:
Add a simple terminal UI for selecting sessions.

Command:
```bash
agentmux tui
````

Use:

* ratatui
* crossterm

TUI requirements:

* Show session list.
* Show status, profile, cwd.
* Enter attaches to selected session.
* `s` stops selected session.
* `k` kills selected session after confirmation.
* `q` quits TUI.
* No split panes yet.
* No full terminal rendering inside TUI yet.

Acceptance:

* User can manage sessions without remembering names.
* TUI remains lightweight.
* Attach still uses existing attach implementation.

After implementation:

* Run `cargo fmt`.
* Run `cargo clippy`.
* Run `cargo test`.
* Manual test.

Summarize changed files and risks.

````

## Phase 10 — Release v0.1

Goal:
Prepare AgentMux v0.1 release.

Tasks:
- Clean README.
- Add install instructions.
- Add example `.agentmux.toml`.
- Add troubleshooting section.
- Add known limitations.
- Add Linux build command.
- Add GitHub Actions CI for:
  - cargo fmt check
  - cargo clippy
  - cargo test
  - cargo build
- Add release binary build instructions.

v0.1 must support:
```bash
agentmux daemon
agentmux status
agentmux profiles
agentmux config print
agentmux run <profile> --name <name> --cwd <path>
agentmux list
agentmux attach <name>
agentmux stop <name>
agentmux kill <name>
agentmux logs <name>
agentmux workspace start
agentmux tui
````

Do not add GUI yet.

After implementation:

* Run all checks.
* Summarize release readiness.

```
```
