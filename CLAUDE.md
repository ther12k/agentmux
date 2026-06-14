# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## First-Time Setup (for AI Agents)

If you are an AI agent (Claude Code, Codex, Aider, etc.) working in this repo for the first time:

### 1. Check prerequisites

```bash
rustc --version   # Must be ≥ 1.70
cargo --version
git --version
```

If Rust is missing:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

### 2. Build

```bash
cargo build --release
```

Binary: `target/release/agentmux`

### 3. Install to PATH

```bash
mkdir -p ~/.local/bin
cp target/release/agentmux ~/.local/bin/agentmux
export PATH="$HOME/.local/bin:$PATH"
```

### 4. Verify

```bash
agentmux version
agentmux doctor
```

### 5. Functional smoke test

```bash
agentmux run shell --name test-session
agentmux list
agentmux logs test-session --tail 20
agentmux stop test-session
```

## Common Development Commands

```bash
# Format code
cargo fmt

# Check linting (run on all targets, warnings are errors)
cargo clippy --all-targets -- -D warnings

# Run tests
cargo test

# Build development binary
cargo build

# Build release binary
cargo build --release

# Install locally
cargo build --release && cp target/release/agentmux ~/.local/bin/agentmux
```

## Architecture Overview

AgentMux is a terminal multiplexer for managing multiple AI coding-agent sessions with a **CLI → Unix socket → Daemon → PTY sessions** architecture.

### Core Components

**CLI (`src/main.rs`, `src/cli.rs`)**
- Entry point using `clap` for command parsing
- All commands auto-start the daemon via `autostart::ensure_daemon_running()`
- Communicates with daemon via Unix socket at `~/.local/share/agentmux/agentmux.sock`

**Daemon (`src/daemon/`)**
- `server.rs`: Long-running daemon process that accepts JSON requests over Unix domain socket
- `protocol.rs`: Defines `Request`/`Response` enums for socket communication
- `state.rs`: Socket path management and daemon running checks
- `session.rs`: `SessionRegistry` manages all sessions, `SessionHandle` holds PTY handles
- `autostart.rs`: Forks daemon process if not already running

**PTY Management (`src/pty/`)**
- `attach.rs`: Terminal raw mode setup, attach/detach logic with Ctrl-b d sequence
- Uses `portable-pty` crate for cross-platform pseudo-terminal operations
- One background reader thread per session fans output to log file + subscriber channel

**Config System (`src/config.rs`, `src/profiles.rs`)**
- Three-layer merge: built-in defaults → global config → project config
- Built-in profiles: pi, codex, gemini, glm, aider, opencode, shell
- Project config: `.agentmux.toml` in project root
- Global config: `~/.config/agentmux/config.toml`

### Session Lifecycle

1. **Spawn**: `agentmux run <profile> --name <name> --cwd <path>` sends `SpawnSession` request to daemon
2. **Registry**: Daemon adds `Session` to `SessionRegistry`, creates PTY, spawns child process
3. **Output**: Background reader thread appends to rotating log + broadcasts to subscriber (if attached)
4. **Attach**: `agentmux attach <name>` connects to PTY, sets terminal raw mode, bridges stdin/stdout
5. **Detach**: Ctrl-b d sequence clears subscriber, restores terminal mode, session continues running
6. **Stop**: `agentmux stop <name>` sends SIGTERM; `agentmux kill <name>` sends SIGKILL

### Critical Patterns

**Socket Protocol**
- All CLI→daemon communication uses **newline-delimited JSON** (one JSON object per line, see `daemon::server::send_request`)
- `AttachSession` is special: after response, connection switches to raw byte forwarding mode

**Attach Coordination**
- Exit channel coordination prevents blocking: main thread waits on `mpsc::Receiver` for either input or output thread to signal completion
- Input thread intentionally NOT joined on detach (may be blocked in `stdin.read()`)
- Resize thread uses `signal_hook::iterator::Signals::forever()` — must call `Handle::close()` on shutdown to prevent hang

**Subscriber Pattern**
- Each session has `Arc<Mutex<Option<Subscriber>>>` for output broadcast
- Only one attached client per session (enforced by `AtomicBool` in `SessionHandle`)
- Subscriber cleared on PTY EOF and on explicit detach

**Config Resolution**
- Profile resolution chain: built-in → global override → project override
- Command existence checked via `which::which()` before spawn
- Missing commands return clear errors before daemon spawn attempt

### Important File Locations

- Socket: `~/.local/share/agentmux/agentmux.sock`
- Logs: `~/.local/share/agentmux/logs/<session-name>.log`
- Log rotation: 10 MB max, 3 rotated files kept

### Testing

Tests are co-located with implementation files (e.g., `src/daemon/session.rs` has extensive `#[cfg(test)]` module). Key test patterns:
- Session lifecycle tests spawn short-lived processes (`true`, `echo`) and poll for exit
- Attach/detach tests use `sleep` commands to simulate long-running sessions
- Subscriber tests verify output forwarding via channels

## Known Limitations

- Linux x86_64 only (uses Linux-specific PTY and epoll APIs)
- One attached client per session (no multi-attach)
- No persistent state across daemon restart (sessions do not survive daemon crash)
- No split panes (use tmux/zellij if needed)

## Platform-Specific Details

The attach mechanism uses raw terminal mode via `libc::tcsetattr`/`tcgetattr`. Terminal size detection uses `libc::ioctl` with `TIOCGWINSZ`. Signal handling uses `signal_hook` crate for SIGWINCH (terminal resize) and cleanup.
