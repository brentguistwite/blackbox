# Ralph Agent Instructions — streak-ambient

You are an autonomous coding agent implementing the `streak-ambient` feature for the `blackbox` CLI. Read `prd.json` in this directory for all stories, priorities, and deps. Never commit to `main` — always work on `feature/streak-ambient`.

---

## Your Task

1. **Read prd.json** — understand all stories before writing any code.
2. **Create feature branch**: `git checkout -b feature/streak-ambient`
3. **Implement P0 stories first** (US-001 through US-004), then P1 (US-005, US-006).
4. **Within each story**: write tests first (TDD), then implement until tests pass.
5. **After each story**: `cargo build` must pass before moving on.
6. **After all stories**: run `cargo test` and `cargo clippy` and fix all failures/warnings.
7. **Batch test fixes** — do not fix failing tests after every iteration during rapid implementation. Fix at the end of each priority tier.
8. **Write a progress report** after each story (see format below).
9. **Do not push** unless the user asks.

---

## Story Dependencies

```
US-005 (config field, no deps)

US-001 (streak_query, no deps)
  └─ US-002 (ActivitySummary.streak_days)
       ├─ US-003 (pretty output)
       │    └─ US-006 (integration test)
       └─ US-004 (JSON output)
            └─ US-006
```

Recommended order: US-005 → US-001 → US-002 → US-003 → US-004 → US-006

---

## Quality Checks

After every story and before declaring done:

```bash
cargo build 2>&1
cargo test  2>&1
cargo clippy -- -D warnings 2>&1
```

Fix all errors and warnings before moving on. Do not use `#[allow(clippy::...)]` unless genuinely inapplicable with explanation.

---

## Project Context

**Crate:** `blackbox-cli` (lib name `blackbox`, binary `blackbox`). Rust edition 2024. SQLite-backed, single crate. All logic is synchronous blocking — do not add `async`.

### Key files

- `src/query.rs` — add `query_streak()` here, alongside `query_activity()`. `ActivitySummary` struct lives here too.
- `src/output.rs` — `render_summary_to_string()`, `JsonSummary` live here. Streak display goes here.
- `src/config.rs` — add `streak_exclude_weekends` field. Use `#[serde(default)]` pattern already present.
- `src/main.rs` — `run_query()` builds `ActivitySummary`. Call `query_streak()` here.
- `src/lib.rs` — module declarations. No new module needed (streak logic lives in query.rs).
- `tests/` — one file per module. New: `tests/streak_test.rs` for US-001 unit tests.

### ActivitySummary struct (current, in src/query.rs)

```rust
pub struct ActivitySummary {
    pub period_label: String,
    pub total_commits: usize,
    pub total_reviews: usize,
    pub total_repos: usize,
    pub total_estimated_time: Duration,
    pub total_ai_session_time: Duration,
    pub repos: Vec<RepoSummary>,
}
```

Add `pub streak_days: u32` field. When adding a field, search all test files for `ActivitySummary {` and update every construction site — the compiler will flag every one.

### query_streak() signature to implement

```rust
/// Returns the current commit streak in calendar days (local time).
/// A streak is consecutive days ending today (or yesterday if no commits today yet)
/// with at least one commit each day.
/// exclude_weekends: if true, Saturday/Sunday are transparent — crossing them does not break the streak.
pub fn query_streak(conn: &Connection, exclude_weekends: bool) -> anyhow::Result<u32>
```

**Algorithm sketch:**

1. Query all distinct local-date days that have at least one commit in `git_activity`:
   ```sql
   SELECT DISTINCT date(timestamp) as day FROM git_activity WHERE event_type = 'commit' ORDER BY day DESC
   ```
   Note: `timestamp` is stored as RFC3339/ISO UTC strings. Use SQLite `date(timestamp, 'localtime')` to convert to local date.

2. Collect the result as a `Vec<NaiveDate>` sorted descending.

3. Determine the "anchor" date: if today (local) is in the set, start counting from today. Otherwise start from yesterday (streak is still alive until EOD).

4. Walk the sorted dates, counting consecutive days. When `exclude_weekends=true`, skip Saturday (weekday 6) and Sunday (weekday 0 in `chrono::Weekday`) — a gap from Friday to Monday counts as consecutive.

5. Return the count. Return 0 if no commits ever or the most recent commit was more than 1 day ago (when exclude_weekends=false) or more than 1 working-day ago (when exclude_weekends=true, e.g. Friday to Monday is OK but Thursday to Monday is not).

**Day boundary:** Use `chrono::Local::now().date_naive()` for today. Use `date(timestamp, 'localtime')` in the SQL query so days align with local time.

### Config field to add (US-005)

```rust
fn default_streak_exclude_weekends() -> bool { false }

// In Config struct:
#[serde(default = "default_streak_exclude_weekends")]
pub streak_exclude_weekends: bool,

// In Config::default():
streak_exclude_weekends: false,
```

No interactive prompt — users opt in by editing `~/.config/blackbox/config.toml` manually.

### run_query() change (src/main.rs)

After building `repos` and before constructing `ActivitySummary`, call:

```rust
let streak_days = blackbox::query::query_streak(&conn, config.streak_exclude_weekends)
    .unwrap_or_else(|e| {
        log::warn!("Failed to compute streak: {}", e);
        0
    });
```

Then include `streak_days` in the `ActivitySummary { ... }` construction.

The `Standup` command path in `main.rs` also constructs `ActivitySummary` — set `streak_days: 0` there (streak is not shown in standup output; no need to compute it).

### Pretty output change (src/output.rs, render_summary_to_string)

In the summary line block (around line 291-299 in current output.rs), after building the main summary string, append the streak suffix when applicable:

```rust
// Only show for "Today" period and when streak > 0
let streak_suffix = if summary.period_label == "Today" && summary.streak_days > 0 {
    let day_word = if summary.streak_days == 1 { "day" } else { "day" }; // always "day" not "days"
    format!("  {}", format!("{}-day streak", summary.streak_days).dimmed())
} else {
    String::new()
};
```

Append `streak_suffix` to the summary line string. The format is: `{N}-day streak` (always "day", not "days" — e.g. "1-day streak", "12-day streak").

**Absolute rule: never include the words `broken`, `missed`, `lost`, `reset`, `end`, or `over` anywhere in streak-related output or strings.** Show only what the user *has* done. If streak is 0, show nothing.

### JSON output change (src/output.rs, JsonSummary)

```rust
#[derive(Serialize)]
pub struct JsonSummary {
    pub period_label: String,
    pub total_commits: usize,
    pub total_reviews: usize,
    pub total_repos: usize,
    pub total_estimated_minutes: i64,
    pub total_ai_session_minutes: i64,
    pub streak_days: u32,   // ADD THIS
    pub repos: Vec<JsonRepo>,
}
```

Populate in `render_json()`:
```rust
let json_summary = JsonSummary {
    // ... existing fields ...
    streak_days: summary.streak_days,
    // ...
};
```

---

## Edge Cases

- **First day ever (one commit, ever):** streak = 1. The algorithm hits `today in set → count = 1, no prior day → return 1`.
- **Streak of 1:** "1-day streak" shown, not "1-days streak". Format string is `"{N}-day streak"` — "day" is always singular in this format.
- **No commits today but yesterday committed:** streak is still alive. The anchor is yesterday. Returns count of consecutive days ending yesterday. Display: `3-day streak` (yesterday is day 3).
- **No commits today, last commit 2 days ago:** streak = 0 (gap broke it). Show nothing in pretty output.
- **Weekend-only developer with exclude_weekends=false (default):** Saturday/Sunday gaps break the streak like any other day. They opted into this by leaving the default.
- **exclude_weekends=true, gap from Friday to Monday:** consecutive, streak continues. Thursday to Monday: gap of 3 days (Fri+Sat+Sun skipped = only 1 working day gap from Thu to Mon = consecutive).
- **exclude_weekends=true, commit on Saturday:** Saturday is a real commit day — it counts toward the streak. "Exclude weekends" means gaps over weekends don't break streaks, not that weekend commits are ignored.
- **All commits in a single UTC day that spans two local days:** SQLite `date(timestamp, 'localtime')` handles this correctly by converting to local time first.
- **DB read error in query_streak:** log warn, return 0. Never panic. Never surface this error to the user in output.
- **ActivitySummary construction in Standup path:** set `streak_days: 0`. Streak is not shown in standup output — no need to compute it there.
- **Week/Month periods:** streak text must not appear. Check `summary.period_label == "Today"` gate in render_summary_to_string.
- **No activity today + no prior history:** ActivitySummary.repos is empty → `render_summary_to_string` returns the "No activity" dimmed line, no streak shown.

---

## Files to Change

### Modified
- `src/query.rs` — add `query_streak()`, add `streak_days: u32` to `ActivitySummary`
- `src/output.rs` — add `streak_days: u32` to `JsonSummary`, update `render_summary_to_string()` and `render_json()`
- `src/config.rs` — add `streak_exclude_weekends: bool` field + default fn
- `src/main.rs` — call `query_streak()` in `run_query()`, set `streak_days: 0` in standup path
- `src/lib.rs` — no new module needed

### New test files
- `tests/streak_test.rs` — US-001 unit tests for `query_streak`

### Additions to existing test files
- `tests/output_test.rs` (or create) — US-003, US-004 unit tests
- `tests/integration_test.rs` or new file — US-006 CLI integration tests

---

## Progress Report Format

After each story output one line:

```
[US-XXX] done — <what was added, 1 sentence>
```

If blocked:

```
[US-XXX] blocked — <reason>
```

After all P0 stories:

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
- Do not add `async` — codebase is fully synchronous.
- **Never output the words `broken`, `missed`, `lost`, `reset`, `end`, or `over` in streak-related strings.** Positive framing only.
- Streak of 0 = show nothing. Streak >= 1 = show `N-day streak` (dimmed) in today pretty output only.
- Edition 2024: `unsafe` block required for `std::env::set_var` in tests.
- Use `tempfile::NamedTempFile` + `open_db` for all DB tests.
- When changing `ActivitySummary`, grep for all construction sites in tests and update them before running `cargo test`.
