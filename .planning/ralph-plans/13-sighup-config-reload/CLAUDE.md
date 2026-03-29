# Agent Instructions: SIGHUP Config Reload

## Project Context

Blackbox is a passive git activity tracking CLI (Rust, edition 2024, SQLite-backed, daemon-based).
Crate name: `blackbox-cli`, binary: `blackbox`. Entry: `src/main.rs`.

Relevant modules:
- `src/daemon.rs` — `start_daemon()`, `run_foreground()`, `stop_daemon()`, `is_daemon_running()`, `PidGuard`. Uses `nix` for signal sends (SIGTERM, process liveness check).
- `src/poller.rs` — `run_poll_loop(config: &Config)`. Main loop: hybrid watcher+poll. Uses `notify` crate watcher + 1s recv timeout, or pure sleep fallback.
- `src/config.rs` — `Config` struct (serde), `load_config()`, `config_dir()`, `data_dir()`. `Config::validate()` enforces `poll_interval_secs >= 10`. `Config::expand_paths()` expands `~`.
- `src/cli.rs` — Clap `Commands` enum, `is_exempt_from_config_check()`.
- `src/main.rs` — match on `Commands` variants, delegates to module functions.

Key dependency: `nix = { version = "0.31.2", features = ["signal", "process", "user"] }` — already in Cargo.toml. Use `nix::sys::signal` for both sending SIGHUP (CLI) and registering handler (daemon).

Signal flag pattern: use `std::sync::atomic::{AtomicBool, Ordering}` + `static RELOAD_FLAG: AtomicBool`. For safe signal handler registration, use `signal_hook` crate OR the `nix` unsafe handler. Prefer `signal_hook` crate (`signal-hook = "0.3"`) with `flag::register` for safe registration; add it to Cargo.toml if needed. If not using signal_hook, use `nix::sys::signal::signal(Signal::SIGHUP, SigHandler::Handler(handler_fn))` with a minimal `extern "C" fn` that sets the atomic.

## 9-Step Task Loop

Work through each user story in order (US-1 → US-7). For each story:

1. **Read** all files that will be modified before touching them.
2. **Branch** — confirm you are on `feature/sighup-config-reload`, not `main`.
3. **Implement** the minimal change required by the story's acceptance criteria.
4. **Build** — `cargo build` must pass before proceeding.
5. **Lint** — `cargo clippy -- -D warnings` must pass.
6. **Test** — write tests (TDD where practical), then `cargo test` must pass.
7. **Review AC** — verify each acceptance criterion is met.
8. **Log** — emit the log lines specified in AC (manually verify format looks right).
9. **Commit** — concise commit message, imperative mood, no sycophancy.

Repeat for each story. Only fix all tests in bulk after finishing all implementation if instructed (per global TDD rules).

## Implementation Notes by Story

### US-1: SIGHUP handler registration

Add to `Cargo.toml` `[dependencies]`:
```toml
signal-hook = "0.3"
```

In `src/poller.rs` (or a new `src/signals.rs`), declare:
```rust
static RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);
```

Register in `run_poll_loop()` before the main loop:
```rust
signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&reload_flag))?;
```
Or use the static directly with `signal_hook::flag::register(SIGHUP, &RELOAD_REQUESTED)`.

`start_daemon()` calls `run_poll_loop()` after fork — registration happens in child, correct.
`run_foreground()` also calls `run_poll_loop()` — same registration, correct.

### US-2: reload_config()

Add `pub fn reload_config() -> anyhow::Result<Config>` to `src/config.rs`. Implementation is identical to `load_config()` — both read the same path. Can be a simple alias or dedup via a private helper. Keep `load_config()` as-is for backward compat.

### US-3: Atomic config swap in poll loop

`run_poll_loop` currently takes `config: &Config`. Change signature to accept owned `Config` (or `Arc<Config>`). Owned is simpler:

```rust
pub fn run_poll_loop(mut config: Config) -> anyhow::Result<()>
```

Update callers in `daemon.rs` (two call sites: `start_daemon` passes owned `config`, `run_foreground` passes owned `config` — both already pass by value to `run_poll_loop(&config)`, so just remove the `&`).

At top of each loop iteration, after `watcher.recv_events(...)` or after `sleep`, check:
```rust
if RELOAD_REQUESTED.swap(false, Ordering::Relaxed) {
    log::info!("SIGHUP received, reloading config");
    match config::reload_config() {
        Ok(new_cfg) => {
            config = new_cfg;
            log::info!("Config reloaded successfully");
            // reset watcher with new watch_dirs
            watcher_opt = RepoWatcher::new(&repos, config.worktree_dir_name.as_deref()).ok();
        }
        Err(e) => log::warn!("Config reload failed: {e}, keeping previous config"),
    }
}
```

`wt_dir_name` local variable (currently used for watcher creation) must be derived from `config` on each use rather than captured once at loop start.

### US-4: Logging

Already embedded in US-3 implementation. Ensure the "changed field summary" log includes old vs new `watch_dirs` and `poll_interval_secs`. Add before swap:
```rust
log::info!("Config reloaded: watch_dirs={:?} poll_interval={}s", new_cfg.watch_dirs, new_cfg.poll_interval_secs);
```

### US-5: `blackbox reload` CLI

In `src/cli.rs`, add to `Commands`:
```rust
/// Send SIGHUP to running daemon to reload config
Reload,
```

In `src/main.rs`, add match arm:
```rust
Commands::Reload => {
    let data_dir = blackbox::config::data_dir()?;
    blackbox::daemon::reload_daemon(&data_dir)?;
}
```

In `src/daemon.rs`, add:
```rust
pub fn reload_daemon(data_dir: &Path) -> anyhow::Result<()> {
    match is_daemon_running(data_dir)? {
        Some(pid) => {
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGHUP,
            )?;
            println!("Reloading config (PID {})", pid);
        }
        None => println!("Daemon not running"),
    }
    Ok(())
}
```

`Reload` should NOT be added to `is_exempt_from_config_check()`.

### US-6: Unit tests for reload_config()

In `tests/config_test.rs` (or wherever config tests live), use `tempfile::tempdir()` to write a TOML file and call `reload_config()` by pointing to a custom path. May need to add a `reload_config_from(path: &Path)` variant or refactor to accept path for testability — preferred since XDG path is non-deterministic in tests.

### US-7: Integration test for `blackbox reload`

In `tests/cli_test.rs` or similar:
```rust
#[test]
fn reload_prints_not_running_when_no_daemon() {
    let data_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    // write a minimal valid config
    // set XDG env vars to tempdir
    let output = Command::cargo_bin("blackbox").unwrap()
        .env("XDG_DATA_HOME", data_dir.path())
        .env("XDG_CONFIG_HOME", config_dir.path())
        .arg("reload")
        .output().unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("Daemon not running"));
}
```

## Documentation Updates
The final story requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## Edge Cases

| Scenario | Expected behavior |
|---|---|
| Config file missing on reload | `reload_config()` returns Err, daemon logs warn, keeps old config |
| Config has TOML parse error on reload | Same — Err, warn, keep old |
| `poll_interval_secs < 10` in new config | `validate()` returns Err, warn, keep old |
| SIGHUP during active `poll_repo()` call | Safe — flag is checked only at loop top, between cycles |
| SIGHUP while daemon sleeps in fallback mode | `sleep` returns; next iteration checks flag immediately |
| SIGHUP while watcher is blocked in `recv_events` | 1s timeout fires; next iteration checks flag |
| Multiple SIGHUPs before loop iteration | `swap(false)` on first check; subsequent signals re-set flag; at most one reload per iteration |
| `blackbox reload` with stale PID file | `is_daemon_running()` already handles stale PIDs (removes file, returns None) |
| SIGHUP changes `watch_dirs` | Watcher is recreated with new dirs; old watches dropped |

## Quality Checks

Run before every commit:
```bash
cargo build
cargo clippy -- -D warnings
cargo test
```

All three must pass cleanly (zero warnings, zero failures).

## File Locations

```
src/config.rs          # reload_config() / reload_config_from()
src/poller.rs          # RELOAD_REQUESTED static, flag check in loop, sig handler registration
src/daemon.rs          # reload_daemon()
src/cli.rs             # Commands::Reload variant
src/main.rs            # match arm for Commands::Reload
Cargo.toml             # signal-hook dependency
tests/config_test.rs   # US-6 unit tests
tests/cli_test.rs      # US-7 integration test
```

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.
