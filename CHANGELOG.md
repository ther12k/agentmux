# Changelog

## v0.1-alpha — 2026-06-14

Initial release.

### Features

- **CLI → Unix socket → Daemon → PTY** architecture
- Daemon auto-starts on first command (no manual `agentmux daemon` needed)
- 7 built-in profiles: pi, codex, gemini, glm, aider, opencode, shell
- Config layering: built-in defaults → global (`~/.config/agentmux/config.toml`) → project (`.agentmux.toml`)
- `agentmux run <profile> --name <name> --cwd <path>` — spawn sessions
- `agentmux attach <name>` — attach to PTY (Ctrl-b d to detach)
- `agentmux send <name> <text> --enter` — programmatic input
- `agentmux logs <name> --tail N` — tail session output
- `agentmux restart <name>` — stop + respawn preserving metadata
- `agentmux stop|kill <name>` — SIGTERM / SIGKILL
- `agentmux workspace start|status|restart-failed` — batch operations
- `agentmux tui` — ratatui-based session switcher
- `agentmux doctor` — diagnostics (TTY, daemon, config, profiles, paths)
- `agentmux config validate` — semantic config validation
- `agentmux version` — version, build mode, target triple
- `agentmux reset --stale|--all` — clean up stale socket / state
- Per-session rotating logs (10 MB max, 3 rotated files kept)
- SIGWINCH resize forwarding during attach
- Human-readable restart markers in logs (chrono)
- CI pipeline (fmt, clippy, test, build)

### Known Limitations

- Linux/macOS only (Unix PTY + signal APIs)
- One attached client per session
- No persistent state across daemon restart
- attach/TUI require a real interactive TTY
- stdin reader thread may remain blocked until keypress after detach
