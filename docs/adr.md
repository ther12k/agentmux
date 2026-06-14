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

---

## ADR-009: Channel-based attach/detach exit coordination

**Date:** 2026-06-14
**Status:** Accepted

### Context

During attach, two threads run concurrently: stdin→socket (input) and socket→stdout (output). The original implementation joined the input thread first. If the PTY exited (output thread got EOF) while the input thread was blocked on `stdin.read()`, attach would hang until the user pressed a key.

### Decision

Use an `mpsc::channel<ExitSide>` for shutdown coordination. Both threads signal on exit. The main thread blocks on `exit_rx.recv()` — wakes when **either** thread exits.

### Details

- Input thread is **detached, not joined** — it may be blocked in `stdin.read()` which cannot be safely interrupted without platform-specific tricks (SIGUSR1 injection or non-blocking stdin).
- After wake: set stop flag → `stream.shutdown(Both)` → `resize_handle.close()` → join output + resize threads.
- Resize thread uses `signal_hook::iterator::Signals::forever()` — must call `Handle::close()` to wake the iterator and prevent hang.

### Rationale

- Channel wakes on whichever side finishes first — no hang on PTY exit.
- Detaching the input thread avoids blocking the caller. Thread terminates naturally on next keypress or process exit.
- Symmetric design: daemon-side `handle_attach` uses the same channel pattern.

### Alternatives considered

- **Join input thread first**: Hangs if PTY exits while stdin blocked.
- **Non-blocking stdin (O_NONBLOCK)**: Platform-specific, fragile, breaks raw mode.
- **SIGUSR1 to interrupt stdin.read()**: Race-prone, signal handler complexity.

---

## ADR-010: Single reader thread + subscriber fan-out

**Date:** 2026-06-14
**Status:** Accepted

### Context

Multiple attach clients could theoretically share a PTY reader. The initial design had per-attach reader threads, causing read conflicts and output stealing.

### Decision

**One** background reader thread per session owns the sole PTY reader. It fans out bytes to:
1. The log file (always, via `RotatingLogWriter`)
2. The current subscriber, if any (set on attach, cleared on detach)

The PTY writer is shared via `Arc<Mutex<Box<dyn Write + Send>>>`.

### Details

- Subscriber slot: `Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>`
- Only one attached client per session (enforced by `AtomicBool`)
- On PTY EOF: reader thread clears subscriber → mpsc channel disconnects → daemon's `pty_to_socket` forwarding thread wakes → attach handler exits cleanly

### Rationale

- Single reader = no read conflicts, no byte duplication
- Subscriber pattern decouples reader lifecycle from attach lifecycle
- Clearing subscriber on EOF prevents zombie "attached" state

### Alternatives considered

- **Per-attach reader**: `try_clone_reader()` called per attach → multiple readers compete for PTY output.
- **Broadcast channel (tokio::broadcast)**: Requires async runtime, overkill for single-client model.

---

## ADR-011: AGENTMUX_DATA_DIR env override for test isolation

**Date:** 2026-06-14
**Status:** Accepted

### Context

Tests that exercise socket path resolution, PID file I/O, and reset logic were touching the developer's real `~/.local/share/agentmux/` directory. This creates flaky tests (daemon might be running) and data loss risk (reset --all could delete real state).

### Decision

`socket_dir()` honors `AGENTMUX_DATA_DIR` environment variable. When set, all paths (socket, PID file, logs) resolve under the override directory instead of the default XDG data dir.

### Test infrastructure

- `tempfile::TempDir` creates isolated temp directories per test
- `static ENV_LOCK: OnceLock<Mutex<()>>` serializes tests that mutate the env var (env vars are process-global)
- Each test saves/restores the previous value

### Rationale

- Zero risk to real user state during testing
- Tests run correctly in parallel (mutex serializes env-var access)
- `AGENTMUX_DATA_DIR` also useful for users who want non-standard data locations

---

## ADR-012: PID file for daemon liveness diagnostics

**Date:** 2026-06-14
**Status:** Accepted

### Context

`agentmux doctor` needs to report whether the daemon process is alive. Socket responsiveness is the primary check, but a stale socket (daemon crashed) is indistinguishable from a slow daemon without PID information.

### Decision

Daemon writes its PID to `<data_dir>/agentmux.pid` on start. Removes it on clean shutdown. Doctor reads the PID file and uses `kill(pid, 0)` to check process liveness.

### PID alive detection

- `kill(pid, 0) == 0` → process exists, we can signal it → **alive**
- `errno == EPERM` → process exists but we lack permission → **alive**
- `errno == ESRCH` → no such process → **not alive**

### Stale PID handling

Doctor detects stale PID files (PID exists but process is dead) and **auto-removes** them. This prevents stale PID accumulation from repeated daemon crashes.

### Rationale

- PID file is best-effort (warns on write/remove failure, never crashes daemon)
- `EPERM` case handles containerized environments where PID 1 may be owned by root
- Auto-cleanup in doctor prevents manual intervention

### Alternatives considered

- **No PID file**: Can't distinguish "daemon slow" from "daemon dead with stale socket"
- **PID in socket metadata**: Would require a successful socket connection (defeats the purpose — if we can connect, daemon is alive)
- **procfs inspection** (`/proc/<pid>/cmdline`): Linux-only, more fragile than `kill(0)`

---

## ADR-013: workspace restart-failed dual-path strategy

**Date:** 2026-06-14
**Status:** Accepted

### Context

`workspace restart-failed` originally sent `RestartSession` for all unhealthy sessions. But `RestartSession` requires existing session metadata in the daemon registry. Missing sessions (never started, or removed after kill) would fail with "Session not found".

### Decision

Split the restart logic into two paths based on session presence in the daemon:

- **Session exists in daemon but exited/failed** → `RestartSession` (reuse stored metadata: command, args, cwd, profile)
- **Session not in daemon at all (missing)** → `SpawnSession` (fresh spawn from workspace config)

### Classification

| Daemon state | Action | Request |
|---|---|---|
| running / attached / detached | SKIPPED | none |
| exited / failed | RESTARTED | `RestartSession` |
| not in daemon | STARTED | `SpawnSession` |
| command not found | FAILED | none |

### Summary output

Reports four counts: `started`, `restarted`, `skipped`, `failed`.

### Rationale

- `RestartSession` preserves original metadata even if workspace config changed since spawn
- `SpawnSession` for missing sessions avoids "not found" errors
- User sees clear distinction between "started fresh" vs "restarted from existing"

### Alternatives considered

- **Always SpawnSession**: Loses session metadata for exited sessions; duplicates session entries
- **Always RestartSession**: Fails for missing sessions
- **Remove then spawn**: Unnecessary churn — restart is cleaner for existing sessions
