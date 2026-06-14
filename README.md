# AgentMux

Terminal multiplexer for managing multiple AI coding-agent sessions. Spawn, attach, send text, and tail logs for agents like `pi`, `codex`, `gemini`, `glm`, `aider`, `opencode`, and more — all from a single CLI.

The daemon auto-starts on first command. No manual `agentmux daemon` needed.

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

## Development

```bash
cargo fmt
cargo clippy --all-targets
cargo test
cargo build
```

---

## License

MIT
