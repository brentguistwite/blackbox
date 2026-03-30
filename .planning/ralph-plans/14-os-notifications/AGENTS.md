# Ralph Agent Instructions — os-notifications

You are an autonomous coding agent implementing the `os-notifications` feature for the `blackbox` CLI. Read prd.json in this directory for the full story list. Implement stories in dependency order. Never commit to `main` — always work on `feature/os-notifications`.

---

## Your Task

1. **Read prd.json** — understand all stories, priorities, deps before writing any code.
2. **Create feature branch**: `git checkout -b feature/os-notifications`
3. **Implement P0 stories first** (US-001 through US-009), then P1 (US-010).
4. **Within each story**: write the test first (TDD), then implement until the test passes.
5. **After each story**: `cargo build` must pass before moving to the next story.
6. **After all stories**: run `cargo test` and `cargo clippy` and fix all failures/warnings.
7. **Do not fix tests after every iteration** during rapid implementation — batch test fixes at the end of each priority tier, or when explicitly asked.
8. **Write a progress report** after completing each story (see format below).
9. **Do not push** unless the user asks.

---

## Story Dependencies

```
US-001 (notify-rust dep)
  └─ US-003 (notifications.rs wrapper)
       └─ US-006 (poller: trigger logic)

US-002 (config fields)
  └─ US-006
  └─ US-010 (integration: config round-trip)

US-004 (notification_log DB table)
  └─ US-006
  └─ US-007 (unit tests: DB helpers)
       └─ US-009 (unit tests: trigger logic)

US-005 (query: daily_summary_for_notification)
  └─ US-006
  └─ US-008 (unit tests: summary query)
       └─ US-009

US-006 ── deps: US-002, US-003, US-004, US-005
US-009 ── deps: US-006, US-007, US-008
```

Recommended implementation order: US-001 → US-002 → US-004 → US-005 → US-003 → US-007 → US-008 → US-006 → US-009 → US-010

---

## Quality Checks

Before declaring done:

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

Fix all clippy warnings. Do not use `#[allow(clippy::...)]` unless genuinely inapplicable with explanation.

---

## Project Context

### Key files to read before writing code

- `src/config.rs` — Config struct and serde default fns pattern. Add fields here.
- `src/db.rs` — migrations array in `open_db()`, existing insert/query patterns. Add migration + `notification_was_sent`, `record_notification_sent` here.
- `src/poller.rs` — `run_poll_loop()` and `full_scan()`. Add `maybe_send_daily_notification()` call after each `full_scan()`.
- `src/query.rs` — `query_activity()`, `global_estimated_time()`, `today_range()`. Add `daily_summary_for_notification()` here.
- `src/enrichment.rs` — OnceLock availability-probe pattern to copy for `is_available()` in notifications.rs.
- `src/lib.rs` — module declarations. Add `pub mod notifications;`.
- `Cargo.toml` — add notify-rust dependency.

### Cargo.toml change

```toml
notify-rust = { version = "4", default-features = false }
```

Add under `[dependencies]` after the existing `notify = "7"` line (that's the filesystem watcher crate — different thing).

### Config fields to add

```rust
fn default_notification_time() -> String {
    "17:00".to_string()
}

// In Config struct:
#[serde(default)]
pub notifications_enabled: bool,
#[serde(default = "default_notification_time")]
pub notification_time: String,

// In Config::default():
notifications_enabled: false,
notification_time: default_notification_time(),
```

### notifications.rs skeleton

```rust
use std::sync::OnceLock;

static NOTIFICATIONS_AVAILABLE: OnceLock<bool> = OnceLock::new();

pub fn is_available() -> bool {
    *NOTIFICATIONS_AVAILABLE.get_or_init(|| {
        // On macOS: notifications always available (no runtime permission denial for basic alerts)
        // On Linux: require DISPLAY or WAYLAND_DISPLAY env var (indicates a desktop session)
        // On other platforms: false
        if cfg!(target_os = "macos") {
            true
        } else if cfg!(target_os = "linux") {
            std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
        } else {
            false
        }
    })
}

pub fn send_notification(title: &str, body: &str) -> anyhow::Result<()> {
    notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show()
        .map(|_| ())
        .map_err(|e| {
            log::warn!("Failed to send OS notification: {}", e);
            anyhow::anyhow!("notification error: {}", e)
        })
}
```

Note: `send_notification` returns `anyhow::Result<()>` but callers should treat errors as non-fatal (log + continue). The function itself logs the warning. Callers should do: `if let Err(e) = notifications::send_notification(...) { log::warn!(...) }`.

### notification_log migration

Add as a new entry in the migrations vec in `db.rs` `open_db()`:

```rust
M::up("CREATE TABLE IF NOT EXISTS notification_log (date TEXT NOT NULL, notification_type TEXT NOT NULL, sent_at TEXT NOT NULL, PRIMARY KEY (date, notification_type));"),
```

Add after the last existing migration.

### DB helper implementations

```rust
pub fn notification_was_sent(
    conn: &Connection,
    date: &str,
    notification_type: &str,
) -> anyhow::Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM notification_log WHERE date = ?1 AND notification_type = ?2)",
        rusqlite::params![date, notification_type],
        |row| row.get(0),
    )?;
    Ok(exists)
}

pub fn record_notification_sent(
    conn: &Connection,
    date: &str,
    notification_type: &str,
) -> anyhow::Result<()> {
    let sent_at = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO notification_log (date, notification_type, sent_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![date, notification_type, sent_at],
    )?;
    Ok(())
}
```

### daily_summary_for_notification() implementation sketch

```rust
pub fn daily_summary_for_notification(
    conn: &Connection,
    session_gap_minutes: u64,
    first_commit_minutes: u64,
) -> anyhow::Result<Option<String>> {
    let (from, to) = today_range();
    let repos = query_activity(conn, from, to, session_gap_minutes, first_commit_minutes)?;

    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    if total_commits == 0 {
        return Ok(None);
    }

    let repo_count = repos.len();
    let total_time = global_estimated_time(&repos, session_gap_minutes, first_commit_minutes);

    let commit_word = if total_commits == 1 { "commit" } else { "commits" };
    let repo_word = if repo_count == 1 { "repo" } else { "repos" };

    let mins = total_time.num_minutes();
    let h = mins / 60;
    let m = mins % 60;
    let time_str = match (h, m) {
        (0, 0) => "< 1m".to_string(),
        (0, _) => format!("{}m", m),
        (_, 0) => format!("{}h", h),
        _ => format!("{}h {}m", h, m),
    };

    Ok(Some(format!(
        "{} {} across {} {} — ~{}",
        total_commits, commit_word, repo_count, repo_word, time_str
    )))
}
```

### maybe_send_daily_notification() implementation sketch

```rust
fn maybe_send_daily_notification(config: &crate::config::Config, conn: &rusqlite::Connection) {
    if !config.notifications_enabled {
        return;
    }
    if !crate::notifications::is_available() {
        return;
    }

    // Parse notification_time as HH:MM
    let parts: Vec<&str> = config.notification_time.split(':').collect();
    let (notify_hour, notify_min) = match parts.as_slice() {
        [h, m] => {
            let h: u32 = match h.parse() {
                Ok(v) => v,
                Err(_) => {
                    log::warn!("Invalid notification_time '{}': bad hour", config.notification_time);
                    return;
                }
            };
            let m: u32 = match m.parse() {
                Ok(v) => v,
                Err(_) => {
                    log::warn!("Invalid notification_time '{}': bad minute", config.notification_time);
                    return;
                }
            };
            (h, m)
        }
        _ => {
            log::warn!("Invalid notification_time format '{}': expected HH:MM", config.notification_time);
            return;
        }
    };

    let now_local = chrono::Local::now();
    let now_time = now_local.time();
    let notify_time = chrono::NaiveTime::from_hms_opt(notify_hour, notify_min, 0)
        .expect("valid HH:MM produces valid NaiveTime");

    if now_time < notify_time {
        return;
    }

    let today_date = now_local.date_naive().to_string(); // "YYYY-MM-DD"

    match crate::db::notification_was_sent(conn, &today_date, "daily_summary") {
        Ok(true) => return,
        Ok(false) => {}
        Err(e) => {
            log::warn!("Failed to check notification_log: {}", e);
            return;
        }
    }

    let body = match crate::query::daily_summary_for_notification(
        conn,
        config.session_gap_minutes,
        config.first_commit_minutes,
    ) {
        Ok(Some(b)) => b,
        Ok(None) => return, // no activity, skip
        Err(e) => {
            log::warn!("Failed to build daily summary for notification: {}", e);
            return;
        }
    };

    if let Err(e) = crate::notifications::send_notification("Blackbox Daily Summary", &body) {
        log::warn!("OS notification failed: {}", e);
        // Still record as sent to avoid retry spam
    }

    if let Err(e) = crate::db::record_notification_sent(conn, &today_date, "daily_summary") {
        log::warn!("Failed to record notification_sent: {}", e);
    }
}
```

Call this after each `full_scan()` in `run_poll_loop()`:

```rust
repos = full_scan(config, &mut repo_states, &conn);
maybe_send_daily_notification(config, &conn);
// ... (also call after the initial full scan before the loop)
```

---

## Edge Cases

- **Notification permission denied (macOS)**: `notify-rust` on macOS uses `NSUserNotification` / `UNUserNotificationCenter`. The `show()` call may fail silently or return an error if the app bundle is not recognized. Handle by logging warn + returning Ok(()) from `send_notification`. Record as sent anyway to avoid spam.
- **Headless / SSH sessions (Linux)**: `is_available()` checks DISPLAY/WAYLAND_DISPLAY. If neither set, returns false and `maybe_send_daily_notification` exits early — no dbus calls made, no panics.
- **Headless / SSH sessions (macOS)**: SSH into macOS won't have a window server session. `notify-rust` show() will fail — caught by the error handler in `send_notification`.
- **Rate limiting**: the `notification_log` table keyed on (date, notification_type) ensures exactly-once per day per type. Even if the daemon restarts multiple times after notification_time, only one notification fires.
- **Daemon restart after notification already sent**: `notification_was_sent` check at the top of `maybe_send_daily_notification` prevents re-send.
- **No activity today**: `daily_summary_for_notification` returns `Ok(None)` → `maybe_send_daily_notification` exits early without writing to `notification_log`. Notification will be re-evaluated on next poll — correct behavior (user might commit later in the day).
- **notification_time parse failure**: log warn + return early. Never panic. Bad value in config = silent degradation.
- **Clashing with `notify` crate**: The existing `notify = "7"` crate in Cargo.toml is the filesystem watcher (`notify::Watcher`). The new `notify-rust = "4"` is for OS notifications (`notify_rust::Notification`). They do not conflict — different crate names and different Rust crate identifiers (`notify` vs `notify_rust`).
- **macOS vs Linux notification appearance**: `notify-rust` handles platform differences internally. On macOS it uses native notification center; on Linux it uses libnotify/dbus. Same Rust API, different backends.
- **Windows**: Not targeted. `is_available()` returns false for non-macOS/non-Linux. `notify-rust` would compile on Windows but we don't test or support it.
- **OnceLock in tests**: `NOTIFICATIONS_AVAILABLE` OnceLock persists for the lifetime of the test process. Test order matters if tests mutate env vars like DISPLAY. Use `#[serial_test::serial]` if env var manipulation is needed, or avoid mutating env vars in notification tests.
- **Rust 2024**: `unsafe` block required for `std::env::set_var` in tests.
- **notify_rust import**: crate name is `notify-rust` (hyphen in Cargo.toml) but Rust identifier is `notify_rust` (underscore). Use `notify_rust::Notification::new()` in code.

---

## Files Changed

### Modified
- `Cargo.toml` — add `notify-rust = { version = "4", default-features = false }`
- `src/lib.rs` — add `pub mod notifications;`
- `src/config.rs` — add `notifications_enabled: bool`, `notification_time: String` fields + defaults
- `src/db.rs` — new migration (notification_log table), add `notification_was_sent`, `record_notification_sent`
- `src/query.rs` — add `daily_summary_for_notification()`
- `src/poller.rs` — add `maybe_send_daily_notification()`, call after each `full_scan()`

### New files
- `src/notifications.rs` — `is_available()`, `send_notification()`

### New test files (or additions to existing)
- `tests/db_test.rs` (or new) — US-007
- `tests/query_test.rs` (or new) — US-008
- `tests/notifications_test.rs` (or new) — US-009, US-010

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

## Documentation Updates
The final story requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Feature branch only.
- Never use `git add -A` or `git add .` — stage specific files by name.
- Do not push unless the user says so.
- All new public types/fns must be `pub`.
- Do not add `async` — entire codebase is synchronous blocking.
- `maybe_send_daily_notification` is private (no `pub`) — only called from within poller.rs.
- `send_notification` errors are always non-fatal: log warn, return Ok(()) or swallow at call site.
- Log at `warn` for unexpected errors, `debug` for "notifications disabled" / "not available" skips.
- Do not add interactive setup prompts for notifications — users opt in by editing config.toml manually.
