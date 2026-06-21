# Contributing to AgentMux

Thanks for your interest in AgentMux. This document covers the basics of contributing.

## Quick start

```bash
git clone https://github.com/ther12k/agentmux
cd agentmux
cargo build
cargo test
```

Requires Rust 1.70 or newer (`rustc --version`).

## Workflow

1. Fork the repo.
2. Create a feature branch from `master`:
   ```bash
   git checkout -b feat/<short-name>
   ```
3. Make your change.
4. Run the verification suite:
   ```bash
   cargo fmt -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```
   All three must pass before opening a PR.
5. Push your branch and open a PR against `master`.
6. Fill in the PR template. Include a one-line summary, motivation, and any breaking changes.
7. Wait for CI to pass and a maintainer to review.

## Commit messages

Use short, descriptive subjects:

```
feat: add `agentmux status --json` output
fix: shutdown handler leaks PTY fd on SIGTERM
docs: clarify `.agentmux.toml` cwd behavior
test: cover workspace restart-failed with mixed session states
chore: bump portable-pty to 0.9.1
```

Avoid bundling unrelated changes.

## Coding conventions

- **Rust style**: `cargo fmt` defaults. Don't fight the formatter.
- **Lints**: CI runs `cargo clippy -- -D warnings`. New code must be clippy-clean.
- **Public API changes**: discuss in an issue first. We try to keep the CLI stable.
- **Dependencies**: each new dep must be justified in the PR description (size, maintenance, license).
- **Tests**: new behavior needs tests. Bug fixes need a regression test that fails before the fix.

## Architecture overview

```
CLI  →  Unix socket  →  Daemon  →  PTY sessions
```

- **CLI** is a stateless clap-based binary that talks to the daemon over a Unix domain socket.
- **Daemon** manages PTY-backed sessions. It auto-starts on first CLI invocation.
- **Sessions** are pseudo-terminals; attaching connects your terminal to the PTY.

Key design decisions live in [`docs/adr.md`](docs/adr.md) (ADR-001 through ADR-013). Read the relevant ADR before proposing changes to daemon lifecycle, session recovery, or IPC protocol.

## Testing tips

- Tests use `AGENTMUX_DATA_DIR` with a temp dir to avoid touching real sessions on the dev machine.
- `attach` needs a real TTY. Don't run attach-tests in CI; verify them on a real terminal via `scripts/manual-tty-test.sh`.
- The `unix_socket` lock test can be flaky if the previous test's daemon didn't shut down. Run with `--test-threads=1` if you see flakes.

## Communication

- **Bug reports**: [open an issue](https://github.com/ther12k/agentmux/issues/new?template=bug_report.yml).
- **Feature requests**: [open an issue](https://github.com/ther12k/agentmux/issues/new?template=feature_request.yml).
- **Security issues**: do NOT open a public issue. Email the maintainer instead (see GitHub profile).

## Code of conduct

Be patient with newcomers. Assume good faith. Critique ideas, not people.

## License

By contributing, you agree that your contributions will be licensed under the MIT License (see [LICENSE](LICENSE)).