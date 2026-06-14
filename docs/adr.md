# AgentMux — Architecture Decision Records (ADR)

Date: 2026-06-14
Status: v0.1 complete

---

## ADR-001: Rust as implementation language

**Date:** 2026-06-14
**Status:** Accepted

### Context

AgentMux manages multiple PTY-based terminal sessions. Requirements: low overhead, fast startup, reliable signal handling, raw terminal I/O, and a long-running daemon.

### Decision

Use **Rust** (edition 2021).

### Rationale

- Zero-cost abstractions → minimal overhead for PTY byte forwarding
- Strong type system catches protocol/serialization errors at compile time
- `portable-pty` crate provides cross-platform PTY abstraction
- `libc` + `nix` give direct access to termios, signals, ioctl
- Single static binary — no runtime dependencies
- Memory safety without GC — important for long-running daemon

### Alternatives considered

- **Go**: Good concurrency, but GC pauses and larger binary. Less ergonomic for raw terminal manipulation.
- **C**: Maximum control, but memory safety risk in a daemon that manages child processes.
- **Node.js/Bun**: Ecosystem has PTY libs (node-pty), but runtime overhead and less predictable signal handling.

---

## ADR-002: Daemon + Unix socket architecture

**Date:** 2026-06-14
**Status:** Accepted

### Context

Sessions must survive CLI exit. The CLI needs to be stateless — spawn, list, attach, stop, kill are independent operations.

### Decision

Split into **CLI client** + **long-running daemon** communicating over a **Unix domain socket** at `~/.local/share/agentmux/agentmux.sock`.

Protocol: **JSON lines** (one JSON object per line, newline-delimited).

```
agentmux CLI
    ↓ (Unix socket, JSON lines)
agentmux daemon
    ↓ (PTY sessions)
agent processes
```

### Rationale

- Unix socket = zero-copy local IPC, no network overhead
- JSON lines = human-readable, easy to debug, language-agnostic
- Daemon owns PTY handles and child processes → sessions persist
- Stateless CLI = any invocation can query/control any session
- Socket file permissions `0o600` = user-only access

### Alternatives considered

- **TCP socket**: Unnecessary network exposure for a local tool. Security risk.
- **Shared memory / mmap**: Faster but complex, harder to debug, no request/response semantics.
- **gRPC**: Overkill for a single-user local tool. Adds protobuf dependency.
- **D-Bus**: Linux-specific, heavy dependency, complex API.

---

## ADR-003: portable-pty 0.9 for PTY management

**Date:** 2026-06-14
**Status:** Accepted

### Context

Need to spawn child processes in pseudo-terminals, resize them, read output, and write input. Must work on Linux (primary), with potential macOS support later.

### Decision

Use the **`portable-pty`** crate (v0.9).

### Rationale

- Abstracts PTY creation across platforms (Linux, macOS, Windows)
- `CommandBuilder` pattern is ergonomic
- Returns `Box<dyn Child + Send + Sync>` — thread-safe child handle
- `MasterPty::try_clone_reader()` and `take_writer()` for I/O access
- `MasterPty::resize()` for terminal resize forwarding

### Key API quirks discovered

- `child.process_id() -> Option<u32>` — NOT `std::process::Child::id()`
- `portable_pty::ExitStatus.exit_code() -> u32` — NOT `std::process::ExitStatus::code() -> Option<i32>`
- `spawn_command()` returns `Box<dyn Child + Send + Sync>` — NOT `std::process::Child`
- `MasterPty` does NOT implement `Clone`
- `SlavePty` must be dropped after spawn so EOF propagates correctly

### Alternatives considered

- **Raw `openpty()` via libc/nix**: More control but platform-specific code, more unsafe.
- **`rustix::pty`**: Lower-level, less documented, fewer examples.
- **`tmux` IPC**: Would tie us to tmux's process model — not standalone.

---

## ADR-004: In-memory session registry with Mutex

**Date:** 2026-06-14
**Status:** Accepted

### Context

The daemon handles concurrent client connections. Each connection may read or modify the session registry (list, add, spawn, stop, kill, reap).

### Decision

Use `Mutex<SessionRegistry>` with a single-threaded accept loop. The registry stores:
- `sessions: HashMap<String, Session>` — serializable session metadata
- `handles: HashMap<String, SessionHandle>` — live `Box<dyn Child + Send + Sync>` handles
- `insertion_order: Vec<String>` — deterministic list ordering

A background reaper thread signals the accept loop every 500ms to reap exited children.

### Rationale

- Single mutex = simple, correct, no lock-ordering bugs
- Accept loop is inherently sequential — one connection at a time
- Reaper polling at 500ms is responsive enough for CLI use without burning CPU
- `insertion_order` ensures deterministic `list` output (HashMap iteration is random)
- Session metadata (serializable) is separated from live handles (not serializable) for clean JSON responses

### Alternatives considered

- **`RwLock`**: Marginal benefit — most operations (spawn, stop, kill) need write access anyway.
- **`tokio` async**: Added complexity with no real benefit for a single-user local daemon. Using std threads.
- **Per-session lock**: Finer granularity but unnecessary at this scale (typically <20 sessions).

---

## ADR-005: Ctrl-b d as detach key sequence

**Date:** 2026-06-14
**Status:** Accepted

### Context

When attached to a session, users need a way to detach without killing the child process. The key must not conflict with common terminal/agent shortcuts.

### Decision

Use **Ctrl-b then d** (same as tmux).

### Rationale

- Familiar to anyone who uses tmux
- Ctrl-b is rare in AI agent TUIs (they typically use Ctrl-c, Ctrl-d, Ctrl-z)
- Two-key sequence avoids accidental detach
- Detected client-side — daemon doesn't need to parse key sequences
- Child process is NOT signaled on detach — it keeps running

### Alternatives considered

- **Ctrl-a d (screen)**: Conflict with bash readline (Ctrl-a = move to line start).
- **Ctrl-\ (SIGQUIT)**: Too aggressive, may terminate agents.
- **Escape key**: Too easy to hit accidentally.
- **`detach` subcommand**: Requires a separate terminal — breaks the attach flow.

---

## ADR-006: Config layering — builtin + global + project

**Date:** 2026-06-14
**Status:** Accepted

### Context

Users need default profiles (pi, codex, shell, etc.), global overrides, and per-project customization.

### Decision

Three-layer merge with later layers winning:

1. **Built-in defaults** (hardcoded in `profiles.rs`)
2. **Global config** (`~/.config/agentmux/config.toml`)
3. **Project config** (`./.agentmux.toml`)

Format: **TOML**.

### Rationale

- TOML is idiomatic for Rust projects (`serde` + `toml` crate)
- Layering follows the principle of least surprise — closest config wins
- Built-in defaults mean zero-config works out of the box
- Project config in `.agentmux.toml` is discoverable and version-controllable
- `HashMap<String, AgentProfile>` merge is trivial — last write wins

### Alternatives considered

- **YAML**: More popular but sensitive to indentation, less ergonomic for this use case.
- **JSON**: No comments, verbose.
- **Single config file only**: Too rigid — can't customize per project.

---

## ADR-007: ratatui + crossterm for TUI

**Date:** 2026-06-14
**Status:** Accepted

### Context

Phase 9 requires a lightweight terminal UI for session management (list, attach, stop, kill).

### Decision

Use **ratatui 0.27** + **crossterm 0.27**.

### Rationale

- ratatui is the maintained successor to tui-rs
- crossterm is cross-platform (Linux + macOS + Windows)
- Immediate-mode rendering model — simple state → screen mapping
- Small binary footprint compared to web-based TUIs
- No async runtime needed — ratatui works with blocking event polling

### Alternatives considered

- **`cursive`**: Higher-level but heavier, opinionated about layout.
- **Web-based (bubbletea)**: Requires Go runtime — not Rust-native.
- **Skip TUI (CLI only)**: Acceptable for v0.1 but poor UX for >5 sessions.

---

## ADR-008: No persistent session state across daemon restart

**Date:** 2026-06-14
**Status:** Accepted (v0.1 limitation)

### Context

When the daemon restarts, all session metadata is lost. Child processes spawned by the old daemon become orphaned.

### Decision

Accept this as a **known limitation for v0.1**. Sessions are in-memory only.

### Rationale

- Persisting session state requires serializing PTY handles, which isn't possible
- Reattaching to orphaned processes requires PID tracking + PTY fd passing — complex
- v0.1 target is single-user local use — daemon restarts are rare
- Logs persist on disk, so output is recoverable

### Future

- v0.2+: Save session metadata to disk on spawn, reattach to PIDs after restart
- Consider `systemd` socket activation for daemon auto-start
