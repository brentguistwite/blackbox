# AGENTS.md

Guidance for AI coding agents working in this repository.

## Workspace
Git worktrees live under `.worktrees/` in project root.

## Rules
Never commit or make changes directly on `main`. Always create a feature branch first.

## Communication Style
BE EXTREMELY CONCISE. Sacrifice grammar for brevity. No sycophantic language. Report facts only. Only break this rule when writing documentation and ADRs.

## Project
Blackbox: flight recorder for your dev day — passive git activity tracking CLI. Rust (edition 2024), SQLite-backed, daemon-based polling.

## Commands
```bash
cargo build                    # Build
cargo test                     # All tests
cargo clippy                   # Lint (if available)
cargo run -- <subcommand>      # Run locally
cargo run -- setup             # Full interactive onboarding
cargo run -- start             # Start polling daemon
cargo run -- today             # Show today's activity
cargo run -- rhythm            # Work rhythm analysis (--days N, --format pretty|json)
```

## Architecture
Single crate. Crate name=`blackbox-cli`, binary=`blackbox` (via [[bin]] in Cargo.toml).
Entry: `src/main.rs` → command dispatch via Clap derive.

```
src/
├── main.rs           # Entry point, command dispatch
├── lib.rs            # Module declarations
├── cli.rs            # Clap CLI definition (Commands enum)
├── claude_tracking.rs # Claude Code session tracking integration
├── config.rs         # Config struct, XDG paths, TOML parsing, run_init()
├── daemon.rs         # Daemon lifecycle (start/stop/status, PID management)
├── db.rs             # SQLite with WAL, migrations, insert/query functions
├── doctor.rs         # Health checks and diagnostics
├── enrichment.rs     # gh CLI integration (OnceLock, graceful degradation)
├── error.rs          # Custom error types (thiserror)
├── git_ops.rs        # poll_repo(), RepoState, commit/branch/merge detection
├── llm.rs            # LLM integration for --summarize flag
├── output.rs         # OutputFormat enum, is_tty(), resolve_format(), render_summary/json/csv
├── poller.rs         # run_poll_loop() — main daemon loop
├── query.rs          # ActivitySummary, RepoSummary, time estimation, date ranges
├── repo_deep_dive.rs # Single-repo deep dive (language breakdown, top files, time, branches, PRs)
├── repo_scanner.rs   # discover_repos() — recursive git repo finder
├── rhythm.rs         # Work rhythm analysis orchestrator (run_rhythm)
├── service.rs        # launchd/systemd install/uninstall (cfg-gated)
├── setup.rs          # Full interactive onboarding wizard
├── shell_hook.rs     # Shell hook generation for zsh/bash/fish
├── tui.rs            # Live TUI dashboard (ratatui)
└── watcher.rs        # Event-driven repo watching (notify crate)

tests/                # Integration tests (one file per module)
```

XDG config: `~/.config/blackbox/config.toml`
XDG data: `~/.local/share/blackbox/` (DB, logs)

## Key Patterns

**Adding a CLI command:**
1. Add variant to `Commands` enum in `cli.rs`
2. Add match arm in `main.rs`
3. Implement logic in appropriate module

**Adding a DB table:**
1. Add migration in `db.rs` `open_db()` migrations array
2. Add insert/query functions in `db.rs`
3. `CREATE TABLE IF NOT EXISTS` pattern

**Adding external tool integration:**
1. Follow `enrichment.rs` pattern (OnceLock for availability caching)
2. Subprocess with timeout (thread + recv_timeout)
3. Graceful degradation: unavailable → return empty/None

**Config changes:**
1. Add field to `Config` in `config.rs` with `#[serde(default)]`
2. Update `run_init()` if interactively configurable

## Testing Patterns
- Integration tests in `tests/` (one file per module)
- `tempfile` for temp dirs, `assert_cmd` for CLI tests
- `git2::Repository::init()` for test repos (not bare mkdir)
- Rust 2024: `unsafe` block required for `std::env::set_var` in tests
- When adding struct fields, update ALL test constructions (compiler catches this)

## Key Dependencies
clap 4.5 (derive), rusqlite 0.38 (bundled), git2 0.20, chrono 0.4, ratatui 0.29, crossterm 0.28, notify 7, reqwest 0.12 (blocking+json), serde+toml, daemonize+nix (daemon/signals), etcetera (XDG), walkdir (fs traversal)
