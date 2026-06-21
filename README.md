<p align="center">
  <strong>AgentMux</strong> — terminal multiplexer for parallel AI coding-agent sessions
</p>

<p align="center">
  <a href="https://github.com/ther12k/agentmux/actions/workflows/ci.yml"><img src="https://github.com/ther12k/agentmux/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/ther12k/agentmux/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License" /></a>
  <a href="https://github.com/ther12k/agentmux/releases"><img src="https://img.shields.io/github/v/release/ther12k/agentmux.svg" alt="latest release" /></a>
</p>

## What it does

AgentMux spawns, attaches, sends text to, and tails logs for multiple AI coding-agent sessions
(`pi`, `codex`, `gemini`, `glm`, `aider`, `opencode`, plus a generic `shell`) from a single CLI.
Built for developers who run several agents in parallel and want a workflow layer over them.

```bash
agentmux workspace start                       # spawn all agents from .agentmux.toml
agentmux send codex-ui "refactor auth" --enter # type into a session without attaching
agentmux logs glm-review --tail 50            # see what an agent is doing
agentmux kill pi-main                          # tear down
```

The daemon auto-starts on first command. No manual `agentmux daemon` needed.

## Why agentmux vs tmux / claude-squad / jmux / agent-deck / kmux

| Tool | Focus | Scripted `send` | First-class logs | Lightweight daemon |
|---|---|---|---|---|
| tmux / zellij | General-purpose window multiplexer | via `send-keys` | via `capture-pane` | foreground only |
| [claude-squad](https://github.com/smtg-ai/claude-squad) | Multi-agent dashboard with web UI | yes | partial | heavier (Go) |
| [jmux](https://github.com/jarredkenny/jmux) | tmux wrapper for agents | manual | wraps tmux | wraps tmux |
| [agent-deck](https://github.com/asheshgoplani/agent-deck) | TUI session manager | no | TUI-only | TUI only |
| [kmux](https://github.com/kkd927/kmux) | Modern TMUX fork for AI era | yes | yes | tmux-based |
| **agentmux** | CLI-first workflow layer | **first-class** | **first-class** | **yes (~100KB)** |

**Pick agentmux when** you want to send prompts to agents from another shell or a CI step,
or tail logs without attaching, or run a single small binary with no Python/web dependency.

**Pick something else when** you want a web dashboard (claude-squad), split panes (tmux/kmux),
or a single TUI window with tabs (agent-deck).

See [Comparison](#comparison) below for the full feature matrix.

---

## Quick Install

**Option A — Cargo (from source):**

```bash
cargo build --release
cp target/release/agentmux ~/.local/bin/agentmux
```

**Option B — Install script:**

```bash
git clone <repo-url> && cd agentmux
chmod +x scripts/install.sh
./scripts/install.sh
```

The script builds a release binary, copies it to `~/.local/bin/agentmux`, creates the log directory at `~/.local/share/agentmux/logs/`, and prints a PATH hint if needed.

---

## Daily Flow

A typical session working with multiple agents:

```bash
# 1. Start a workspace from a project config
cd my-project
agentmux workspace start

# 2. See what's running
agentmux list

# 3. Attach to your main agent
agentmux attach pi-main
# ... interact with the agent ...
# Detach with Ctrl-b d (session keeps running)

# 4. Send a quick command without attaching
agentmux send codex-ui "refactor the auth module" --enter

# 5. Tail logs to see what an agent is doing
agentmux logs glm-review --tail 50

# 6. Stop a specific session
agentmux stop shell

# 7. Kill it entirely (removes the session)
agentmux kill pi-main

# 8. Or use the TUI for a dashboard view
agentmux tui
```

---

## Commands

- **`agentmux workspace start`** — Batch-spawn sessions from `.agentmux.toml`
- **`agentmux workspace status`** — Show workspace session health
- **`agentmux workspace restart-failed`** — Start missing sessions + restart exited/failed sessions
- **`agentmux list`** — Show all active sessions
- **`agentmux attach <name>`** — Attach to a session (Ctrl-b d to detach)
- **`agentmux stop <name>`** — Stop a session (PTY closed, process terminated)
- **`agentmux send <name> <text> --enter`** — Send text to a session's stdin
- **`agentmux logs <name> --tail N`** — Show last N lines of session output
- **`agentmux kill <name>`** — Force-kill and remove a session
- **`agentmux tui`** — Interactive terminal UI

---

## Configuration

### Global config

`~/.config/agentmux/config.toml` — optional global defaults:

```toml
[defaults]
log_level = "info"
```

### Project workspace config

Place `.agentmux.toml` in your project root. On `agentmux workspace start`, every session in the file is spawned at once:

```toml
[workspace]
name = "my-project"

[agents.pi]
command = "pi"
args = []

[agents.codex]
command = "codex"
args = []

[agents.glm]
command = "glm"
args = []

[agents.shell]
command = "bash"
args = []

[[workspace.sessions]]
name = "pi-main"
profile = "pi"
cwd = "."

[[workspace.sessions]]
name = "codex-ui"
profile = "codex"
cwd = "."

[[workspace.sessions]]
name = "glm-review"
profile = "glm"
cwd = "."

[[workspace.sessions]]
name = "shell"
profile = "shell"
cwd = "."
```

A full example is in [`examples/pi-workspace.agentmux.toml`](examples/pi-workspace.agentmux.toml).

### Built-in Profiles

- **pi** — `pi`
- **codex** — `codex`
- **gemini** — `gemini`
- **glm** — `glm`
- **aider** — `aider`
- **opencode** — `opencode`
- **shell** — `bash`

---

## Architecture

```
CLI  →  Unix socket  →  Daemon  →  PTY sessions
```

- **CLI** sends commands to the daemon over a Unix domain socket.
- **Daemon** manages PTY-backed sessions. It auto-starts on first CLI invocation if not already running.
- **Sessions** are pseudo-terminals; attaching connects your terminal to the PTY.

### Paths

- **Unix socket:** `~/.local/share/agentmux/agentmux.sock`
- **Session logs:** `~/.local/share/agentmux/logs/<name>.log`

### Log Rotation

- Max file size: **10 MB**
- Rotated files kept: **3**
- When a log hits 10 MB it is rotated to `<name>.log.1`, older copies shift up, and the oldest is deleted.

### Detach Key

**Ctrl-b d** — detaches from an attached session and returns you to your shell. The session continues running in the background.

---

## Known Limitations

- **Linux/macOS only** — uses Unix-specific PTY and signal APIs. Windows unsupported.
- **One attached client per session** — simultaneous multi-attach is not supported.
- **attach/TUI require a real interactive TTY** — will not work in piped/non-interactive shells.
- **stdin reader thread may remain blocked** until next keypress or process exit after detach.
- **No persistent state across daemon restart** — sessions do not survive a daemon crash or manual kill. Restart and re-spawn.
- **No split panes** — each session is a single full-screen terminal. Use tmux/zellij if you need panes.
- **No browser preview, no GUI** — CLI and TUI only.

---

## v0.1-alpha Quick Start

```bash
# Check your environment
agentmux doctor

# Validate config
agentmux config validate

# Start workspace sessions
agentmux workspace start

# Check workspace status
agentmux workspace status

# Attach to a session
agentmux attach pi-main

# Restart a crashed session
agentmux restart pi-main

# Restart all failed sessions
agentmux workspace restart-failed

# Check version
agentmux version
```

---

## Troubleshooting

### Broken terminal after detach

If your terminal looks garbled after detaching:

```bash
reset
stty sane
```

### Stale socket (daemon won't start)

**Preferred:** Use the built-in reset command:

```bash
agentmux reset --stale
```

**Fallback** (manual socket removal — only if reset doesn't work):

```bash
rm ~/.local/share/agentmux/agentmux.sock
```

Then run any `agentmux` command — the daemon will auto-start fresh.

### Daemon won't start

- Check that `~/.local/share/agentmux/` exists and is writable.
- Check for another process holding the socket:
  ```bash
  lsof ~/.local/share/agentmux/agentmux.sock
  ```
- Kill any stale process, remove the socket, and retry.

---

## AI Agent Setup Guide

When an AI coding agent (Claude Code, Codex, Aider, etc.) needs to set up and build AgentMux in a fresh environment, follow these exact steps.

### Prerequisites

The environment must have:

- **Rust toolchain** (stable, ≥ 1.70): `rustup show` or `rustc --version`
- **Cargo**: comes with Rust
- **Git**
- **Linux or macOS** (Windows unsupported)

If Rust is missing, install it:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

### Build from source

```bash
git clone https://github.com/ther12k/agentmux.git
cd agentmux
cargo build --release
```

The binary will be at `target/release/agentmux`.

### Install to PATH

```bash
mkdir -p ~/.local/bin
cp target/release/agentmux ~/.local/bin/agentmux
export PATH="$HOME/.local/bin:$PATH"
# Add to shell profile for persistence:
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
```

### Verify the install

```bash
agentmux version
agentmux doctor
```

`doctor` checks:
- Rust/Cargo availability
- Socket path writability
- TTY availability
- PID file state

### Quick functional test

```bash
# Spawn a shell session
agentmux run shell --name test-session

# List sessions
agentmux list

# Check session output
agentmux logs test-session --tail 20

# Stop the session
agentmux stop test-session
```

### Development workflow

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

All four must pass before committing. The project has 98 tests.

### Project structure for agents

```
src/
├── main.rs              # Entry point
├── cli.rs               # All clap commands and dispatch
├── config.rs            # Config loading, merge, validation
├── profiles.rs          # 7 built-in agent profiles
├── autostart.rs         # Daemon auto-fork on first command
├── doctor.rs            # Environment diagnostics
├── workspace.rs         # Batch workspace start/status/restart
├── tui.rs               # Ratatui terminal UI
├── daemon/
│   ├── server.rs        # Unix socket server, request dispatch
│   ├── protocol.rs      # Request/Response JSON enums
│   ├── session.rs       # SessionRegistry, PTY management
│   └── state.rs         # Socket/PID path helpers
├── pty/
│   ├── mod.rs           # Foreground PTY spawn (experimental)
│   └── attach.rs        # Attach/detach coordination
└── storage/
    └── logs.rs          # Rotating log writer (10MB, 3 files)
```

### Key conventions for agents editing this codebase

1. **Never use `unwrap()` on user-facing paths** — use `?` or `let Some(x) else { ... }`.
2. **Tests must not touch real `~/.local/share/agentmux`** — use `AGENTMUX_DATA_DIR=/tmp/...` or `tempfile::TempDir`.
3. **`reset --all` must never delete logs or kill processes.**
4. **Socket protocol is newline-delimited JSON** (not length-prefixed).
5. **One attached client per session** — enforced by `AtomicBool`.
6. **All gates must pass before commit:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo build --release`.

### Environment variables

| Variable | Purpose |
|---|---|
| `AGENTMUX_DATA_DIR` | Override data directory (for tests/isolation) |
| `AGENTMUX_SOCKET_PATH` | Override Unix socket path |

### If an agent gets stuck

- **Build fails:** Check Rust version (`rustc --version` ≥ 1.70), run `cargo clean && cargo build`.
- **Tests fail on socket/PID:** Ensure tests use `AGENTMUX_DATA_DIR` with a temp dir, not real state.
- **Attach hangs in CI/container:** Expected — attach needs a real TTY. Use `scripts/manual-tty-test.sh` on a real terminal.
- **`agentmux` command not found:** Check `~/.local/bin` is in `$PATH`.

---

## Comparison

### Tool focus

| Tool | What it does | When to use it |
|---|---|---|
| tmux / zellij | General-purpose terminal multiplexer (windows, panes, sessions) | You want a window manager for any terminal work, not just AI agents |
| [claude-squad](https://github.com/smtg-ai/claude-squad) | Multi-agent CLI orchestrator with web UI (Go) | You want a full agent-management app with browser dashboard |
| [jmux](https://github.com/jarredkenny/jmux) | tmux-based parallel agent runner | You already know tmux and want a thin agent-aware layer over it |
| [agent-deck](https://github.com/asheshgoplani/agent-deck) | TUI session manager for AI agents | You want a single TUI window with tabbed agent sessions |
| [kmux](https://github.com/kkd927/kmux) | Modern TMUX fork for the AI era | You want tmux-style keybindings with first-class agent support |
| **agentmux** | CLI-first workflow layer: spawn, send, tail, kill from one binary | You want scripted/orchestrated access to multiple agents from other shells or CI |

### Feature matrix (snapshot 2026-06-21)

| Feature | agentmux | claude-squad | tmux |
|---|---|---|---|
| Spawn PTY-backed session | ✓ | ✓ | ✓ |
| Attach/detach from session | ✓ | ✓ | ✓ |
| Send text to session stdin | ✓ (scriptable) | ✓ (scriptable) | ✓ (via `send-keys`) |
| Tail session output | ✓ | partial | ✓ (via `capture-pane`) |
| Auto-rotate session logs | ✓ (10MB × 3) | varies | ✗ (manual) |
| Web UI | ✗ (intentional) | ✓ | ✗ |
| Split panes | ✗ (intentional) | partial | ✓ |
| Multi-host / remote agents | ✗ | partial | ✓ |
| Cross-platform (Windows) | ✗ (Linux/macOS only) | partial | ✓ (via WSL/Cygwin) |

This matrix is a snapshot. Verify before relying on specifics.

---

## License

MIT — see [LICENSE](LICENSE).
