# Ralph Agent Instructions — richer-status

You are an autonomous coding agent implementing the `richer-status` feature for the `blackbox` CLI. Read prd.json in this directory for the full story list. Implement stories in dependency order. Never commit to `main` — always work on `feature/richer-status`.

---

## Your Task

Follow these steps in order:

1. **Read prd.json** — understand all stories, priorities, deps before writing any code.
2. **Create feature branch**: `git checkout -b feature/richer-status`
3. **Implement P0 stories first** (US-001 through US-009 and US-011), then P1 (US-010).
4. **Within each story**: write the test first (TDD), then implement until the test passes.
5. **After each story**: `cargo build` must pass before moving to the next story.
6. **After all stories**: run `cargo test` and `cargo clippy` and fix all failures/warnings.
7. **Do not fix tests after every iteration** during rapid implementation — batch test fixes at the end of each priority tier, or when explicitly asked.
8. **Write a progress report** after completing each story (see format below).
9. **Do not push** unless the user asks.

---

## Story Dependencies

```
US-001 (daemon_state table + set/get)
  ├─ US-002 (poller: write last_poll_at + repos_watched)
  ├─ US-003 (DaemonStatus struct + get_daemon_status)
  │    ├─ US-004 (daemon_status() + --format flag)
  │    │    └─ US-005 (pretty output with health indicator)
  │    │         └─ US-010 (integration test: CLI)
  │    ├─ US-007 (unit tests: DaemonStatus)
  │    └─ US-011 (serde derives)
  └─ US-006 (count_events_today helper)
       └─ US-007
       └─ US-008 (unit tests: count_events_today)

US-009 (unit tests: set/get daemon_state) — deps: US-001
```

Recommended implementation order: US-001 → US-006 → US-009 → US-008 → US-011 → US-003 → US-007 → US-002 → US-004 → US-005 → US-010

---

## Quality Checks

Before declaring done, all three must pass cleanly:

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

Fix all clippy warnings. Do not use `#[allow(clippy::...)]` unless the lint is genuinely inapplicable and you explain why.

---

## Project Context

### Key files to read before writing code

- `src/daemon.rs` — existing `daemon_status()`, `is_daemon_running()`, `pid_file_path()`. You will add `DaemonStatus`, `HealthIndicator`, and `get_daemon_status()` here.
- `src/db.rs` — all DB functions and the migrations array in `open_db()`. Add new migration + `set_daemon_state`, `get_daemon_state`, `count_events_today` here.
- `src/poller.rs` — `run_poll_loop()` and `full_scan()`. Write last_poll_at after every full_scan() call.
- `src/cli.rs` — `Commands::Status` variant. Add `format: OutputFormat` arg.
- `src/main.rs` — match arm for `Commands::Status`. Pass format to daemon_status().
- `src/output.rs` — `OutputFormat` enum (already has Pretty/Json/Csv). Import it for daemon_status().
- `tests/` — look at existing test files (e.g. tests/db_test.rs if present) for `setup_db()` patterns using tempfile.

### Existing daemon_status() — what you're replacing

```rust
// src/daemon.rs (current)
pub fn daemon_status(data_dir: &Path) -> anyhow::Result<()> {
    match is_daemon_running(data_dir)? {
        Some(pid) => println!("Running (PID {})", pid),
        None => println!("Stopped"),
    }
    Ok(())
}
```

Replace with a call to `get_daemon_status()` then dispatch to pretty/JSON rendering based on `format`.

### New DaemonStatus struct

```rust
#[derive(Debug, serde::Serialize)]
pub enum HealthIndicator { Green, Yellow, Red }

#[derive(serde::Serialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub last_poll_at: Option<chrono::DateTime<chrono::Utc>>,
    pub repos_watched: Option<u64>,
    pub db_size_bytes: Option<u64>,
    pub events_today: Option<u64>,
    pub health: HealthIndicator,
}
```

### get_daemon_status() implementation sketch

```rust
pub fn get_daemon_status(data_dir: &Path) -> anyhow::Result<DaemonStatus> {
    let pid = is_daemon_running(data_dir)?;
    let running = pid.is_some();

    // Uptime: mtime of PID file
    let uptime_secs = if running {
        let pid_path = pid_file_path(data_dir);
        std::fs::metadata(&pid_path).ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs())
    } else { None };

    // Try to open DB — gracefully if absent
    let db_path = data_dir.join("blackbox.db");
    let db_size_bytes = std::fs::metadata(&db_path).ok().map(|m| m.len());

    let (last_poll_at, repos_watched, events_today) = if db_path.exists() {
        match crate::db::open_db(&db_path) {
            Ok(conn) => {
                let lp = crate::db::get_daemon_state(&conn, "last_poll_at").ok().flatten()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));
                let rw = crate::db::get_daemon_state(&conn, "repos_watched").ok().flatten()
                    .and_then(|s| s.parse::<u64>().ok());
                let et = crate::db::count_events_today(&conn).ok();
                (lp, rw, et)
            }
            Err(_) => (None, None, None),
        }
    } else {
        (None, None, None)
    };

    let health = compute_health(running, last_poll_at);

    Ok(DaemonStatus { running, pid, uptime_secs, last_poll_at, repos_watched, db_size_bytes, events_today, health })
}

fn compute_health(running: bool, last_poll_at: Option<chrono::DateTime<chrono::Utc>>) -> HealthIndicator {
    if !running { return HealthIndicator::Red; }
    match last_poll_at {
        None => HealthIndicator::Yellow,
        Some(t) => {
            let age = chrono::Utc::now().signed_duration_since(t);
            if age <= chrono::Duration::minutes(5) { HealthIndicator::Green }
            else if age <= chrono::Duration::minutes(30) { HealthIndicator::Yellow }
            else { HealthIndicator::Red }
        }
    }
}
```

### Pretty output implementation sketch

```rust
fn render_status_pretty(status: &DaemonStatus) {
    use colored::Colorize;
    let (icon, label) = match status.health {
        HealthIndicator::Green  => ("✓".green().bold(),  "Running".green().bold()),
        HealthIndicator::Yellow => ("⚠".yellow().bold(), "Running (stale)".yellow().bold()),
        HealthIndicator::Red    => ("✗".red().bold(),    "Stopped".red().bold()),
    };
    println!("{} {}", icon, label);
    if let Some(pid) = status.pid {
        println!("  PID:           {}", pid);
    }
    if let Some(secs) = status.uptime_secs {
        println!("  Uptime:        {}", format_uptime(secs));
    }
    // last_poll_at
    match status.last_poll_at {
        Some(t) => {
            let age = chrono::Utc::now().signed_duration_since(t);
            println!("  Last poll:     {} ago", format_duration_ago(age));
        }
        None => println!("  Last poll:     never"),
    }
    // repos_watched
    match status.repos_watched {
        Some(n) => println!("  Repos watched: {}", n),
        None    => println!("  Repos watched: unknown"),
    }
    // db_size_bytes
    match status.db_size_bytes {
        Some(b) => println!("  DB size:       {:.1} KB", b as f64 / 1024.0),
        None    => println!("  DB size:       no DB yet"),
    }
    // events_today
    println!("  Events today:  {}", status.events_today.unwrap_or(0));
}

fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    match (h, m) {
        (0, 0) => format!("{}s", s),
        (0, _) => format!("{}m {}s", m, s),
        _      => format!("{}h {}m", h, m),
    }
}
```

### daemon_state migration

Add as a new entry in the migrations vec in db.rs `open_db()`:

```rust
M::up("CREATE TABLE IF NOT EXISTS daemon_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);"),
```

Add after the last existing migration. The migration index increments automatically via rusqlite_migration.

### count_events_today SQL

```rust
pub fn count_events_today(conn: &Connection) -> anyhow::Result<u64> {
    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0).unwrap()
        .and_utc()
        .to_rfc3339();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM git_activity WHERE timestamp >= ?1",
        rusqlite::params![today_start],
        |row| row.get(0),
    )?;
    Ok(count as u64)
}
```

### poller.rs: write heartbeat after full_scan

In run_poll_loop(), after each `full_scan()` call:

```rust
repos = full_scan(config, &mut repo_states, &conn);
let now = chrono::Utc::now().to_rfc3339();
if let Err(e) = db::set_daemon_state(&conn, "last_poll_at", &now) {
    log::warn!("Failed to write last_poll_at: {}", e);
}
if let Err(e) = db::set_daemon_state(&conn, "repos_watched", &repos.len().to_string()) {
    log::warn!("Failed to write repos_watched: {}", e);
}
```

There are two places full_scan() is called: the initial call before the loop, and inside the loop (both watcher and polling paths). Write after all of them.

### CLI change: Commands::Status with --format

In cli.rs, change:

```rust
/// Show daemon status (running/stopped)
Status,
```

to:

```rust
/// Show daemon status (running/stopped)
Status {
    /// Output format: pretty, json
    #[arg(long, default_value = "pretty")]
    format: OutputFormat,
},
```

Note: CSV doesn't make sense for status; accept it without error but treat same as pretty (or just let OutputFormat::Csv fall through to pretty). The acceptance criteria requires JSON to work; CSV behavior is unspecified.

In main.rs update the match arm:

```rust
Commands::Status { format } => {
    let data_dir = blackbox::config::data_dir()?;
    blackbox::daemon::daemon_status(&data_dir, format)?;
}
```

Also exempt `Commands::Status { .. }` from config check in `is_exempt_from_config_check()` — status should work even without a config file:

```rust
pub fn is_exempt_from_config_check(&self) -> bool {
    matches!(
        self,
        Commands::Init { .. }
            | Commands::Status { .. }   // <-- add this
            | Commands::Setup
            | Commands::Completions { .. }
            | Commands::Hook { .. }
            | Commands::RunForeground
            | Commands::NotifyDir { .. }
    )
}
```

---

## Documentation Updates
The final story (US-012) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## Edge Cases

- **Daemon not running**: pid=None, uptime_secs=None, health=Red. Still show last_poll_at, repos_watched, DB size, events_today from DB if DB exists.
- **Stale PID file**: is_daemon_running() already handles this — it calls `kill(pid, None)` and removes the stale file if the process is dead. get_daemon_status() just calls is_daemon_running(), so stale PID → running=false automatically.
- **DB doesn't exist yet (fresh install)**: db_path.exists() returns false → all DB-derived fields are None. Output: `✗ Stopped`, `Last poll: never`, `Repos watched: unknown`, `DB size: no DB yet`, `Events today: 0`.
- **DB exists but daemon_state table missing** (old DB from before this migration): open_db() runs migrations, so daemon_state table will be created. get_daemon_state("last_poll_at") returns Ok(None) since no rows yet. last_poll_at=None → health=Yellow if running, Red if stopped.
- **DB exists but git_activity table missing**: impossible — it's migration 1. But count_events_today returns Ok(0) on empty table.
- **DB locked by daemon**: open_db() sets busy_timeout=5000ms via pragma. On timeout, Err → all DB fields None, no panic.
- **uptime_secs from PID file mtime**: PID file is written by daemonize crate or PidGuard::new(). Its mtime approximates daemon start time. Accuracy is within the OS mtime resolution (1s). If mtime is in the future (clock skew), elapsed() returns Err → uptime_secs=None.
- **last_poll_at in the future** (clock skew): `signed_duration_since` can be negative. In compute_health, if age < 0 treat as Green (daemon just polled, clock skew is benign).
- **Rust 2024**: `unsafe` block required for `std::env::set_var` in tests.
- **When adding Commands::Status { format }**: the match arm in main.rs changes from `Commands::Status =>` to `Commands::Status { format } =>`. Update `is_exempt_from_config_check` too — it now needs `Commands::Status { .. }`.
- **OutputFormat import in daemon.rs**: daemon.rs doesn't currently use OutputFormat. Import `use crate::output::OutputFormat;` at the top of daemon.rs, or pass format as a string. Prefer the typed enum.

---

## Files Changed

### Modified
- `src/daemon.rs` — add DaemonStatus, HealthIndicator, get_daemon_status(), render_status_pretty(), update daemon_status() signature and body
- `src/db.rs` — new migration (daemon_state table), add set_daemon_state, get_daemon_state, count_events_today
- `src/poller.rs` — write last_poll_at + repos_watched after each full_scan()
- `src/cli.rs` — Commands::Status gets format: OutputFormat field; is_exempt_from_config_check updated
- `src/main.rs` — match arm for Commands::Status updated

### New test files (or additions to existing)
- `tests/daemon_test.rs` (or add to existing) — US-007
- `tests/db_test.rs` (or add to existing) — US-008, US-009

---

## Progress Report Format

After each story, output one line:

```
[US-XXX] done — <what was added, 1 sentence>
```

If blocked:

```
[US-XXX] blocked — <reason>
```

After completing all P0 stories:

```
=== P0 complete. cargo build: OK. Tests: N passing, M failing (will fix at end). ===
```

---

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Feature branch only.
- Never use `git add -A` or `git add .` — stage specific files by name.
- Do not push unless the user says so.
- All new public types/fns must be `pub`.
- Do not add new crate dependencies — all required crates (colored, chrono, serde, serde_json, rusqlite, nix) are already in Cargo.toml.
- Do not add `async` — entire codebase is synchronous blocking.
- Log at `warn` for unexpected errors in poller heartbeat writes, `debug` for "DB not found" skips.
- When changing `Commands::Status` from a unit variant to a struct variant, search ALL match arms and `matches!` calls for `Commands::Status` and update them — the compiler catches these but be proactive.
