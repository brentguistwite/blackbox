# Ralph Agent Instructions — work-rhythm-insights

You are an autonomous coding agent implementing the `work-rhythm-insights` feature for the `blackbox` CLI. Read prd.json in this directory for the full story list. Implement stories in dependency order. Do not commit to `main` — always work on `feature/work-rhythm-insights`.

---

## Your Task

Follow these steps in order:

1. **Read prd.json** — understand all stories, priorities, and deps before writing any code.
2. **Create feature branch**: `git checkout -b feature/work-rhythm-insights`
3. **Implement P0 stories first** (US-001, US-002, US-003, US-004, US-011, US-012, US-013, US-014, US-018), then P1 (US-005–US-008, US-015, US-016), then P2 (US-009, US-010, US-017).
4. **Within each story**: write the test first (TDD), then implement until the test passes.
5. **After each story**: `cargo build` must pass before moving to the next story.
6. **After all stories**: run `cargo test` and `cargo clippy` and fix all failures/warnings.
7. **Do not fix tests after every iteration** during rapid implementation — batch test fixes at the end of each priority tier, or when explicitly asked.
8. **Write a progress report** after completing each story (see format below).
9. **Do not push** unless the user asks.

---

## Story Dependencies

```
US-001 (hour histogram query)
  └─ US-002 (hour histogram display)
       └─ US-012 (render_rhythm)
            └─ US-011 (CLI: blackbox rhythm)
                  └─ US-018 (CLI integration test)

US-003 (DOW histogram query)
  └─ US-004 (DOW histogram display)
       └─ US-012

US-005 (after-hours query)
  └─ US-006 (after-hours display)
       └─ US-012

US-007 (session distribution query)
  └─ US-008 (session distribution display)
       └─ US-012

US-009 (burst pattern query)
  └─ US-010 (burst pattern display)
       └─ US-012

US-013 depends on US-001
US-014 depends on US-003
US-015 depends on US-005
US-016 depends on US-007
US-017 depends on US-009
```

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

- `src/cli.rs` — Commands enum, clap derive pattern. Add `Rhythm` variant here.
- `src/main.rs` — match arm dispatch. Add arm for `Commands::Rhythm { days, format }`.
- `src/query.rs` — all DB query functions live here. Add rhythm query fns here.
- `src/output.rs` — all render functions live here. Add rhythm render fns here.
- `src/db.rs` — SQLite migration pattern (rusqlite_migration). No new tables needed for this feature — all data is already in `git_activity`.
- `tests/query_test.rs` — integration test pattern: `setup_db()` returns `(Connection, NamedTempFile)`, helpers like `insert_activity(...)`.

### DB schema (relevant table)

```sql
CREATE TABLE git_activity (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_path TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK(event_type IN ('commit','branch_switch','merge')),
    branch TEXT,
    commit_hash TEXT,
    author TEXT,
    message TEXT,
    timestamp TEXT NOT NULL,   -- RFC3339 UTC string
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

Timestamps are RFC3339 UTC strings. Use `chrono::DateTime::parse_from_rfc3339` + `.with_timezone(&Utc)` to parse, then `.with_timezone(&Local)` to convert to local time for bucketing.

### Existing patterns to follow

**Query function signature pattern:**
```rust
pub fn commit_hour_histogram(
    conn: &Connection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<[u32; 24]>
```

**Chrono local hour extraction:**
```rust
use chrono::{Local, Timelike, Datelike};
let local_dt = utc_dt.with_timezone(&Local);
let hour = local_dt.hour() as usize;  // 0–23
let weekday_idx = local_dt.weekday().num_days_from_monday() as usize; // 0=Mon..6=Sun
```

**CLI variant pattern (cli.rs):**
```rust
/// Show work rhythm patterns (commit timing analysis)
Rhythm {
    /// Number of days to analyze (default 30)
    #[arg(long, default_value_t = 30)]
    days: u64,
    /// Output format: pretty, json
    #[arg(long, default_value = "pretty")]
    format: OutputFormat,
},
```

**main.rs dispatch pattern:**
```rust
Commands::Rhythm { days, format } => {
    blackbox::rhythm::run_rhythm(days, format)?;
}
```

Consider adding `src/rhythm.rs` as a new module to hold the `run_rhythm()` orchestration fn (load config, open DB, call query fns, render). Register it in `src/lib.rs`.

**Test file convention:** create `tests/rhythm_test.rs`. Import from `blackbox::` crate. Use `setup_db()` pattern from query_test.rs (copy the helper or factor it out — see below).

**Test helper duplication:** `setup_db()` is defined locally in query_test.rs. Either copy it into rhythm_test.rs or add a `tests/helpers.rs` module with `pub fn setup_db()`. The latter is cleaner if > 2 test files need it.

### OutputFormat enum

Located in `src/output.rs`. Already has `Pretty`, `Json`, `Csv`. The rhythm command only needs `Pretty` and `Json` (no CSV). Validate in the command handler and return an error for `--format csv`.

### Rendering style

Follow `render_summary_to_string` style: build `Vec<String>` lines, join with `\n`. Use `colored` crate for terminal color. Header: `.bold().cyan()`. Section labels: `.bold()`. Bar charts: use `colored::*` — bars in yellow, peak in green.

ASCII bar chart target format:
```
Hour of day (last 30 days)
  0 |                        0
  1 |                        0
  ...
  9 | ████████████           18
 10 | ████████████████████   24  <- peak
 11 | ████████████████       20
  ...
 23 |                        0
Peak: 10:00–11:00 (24 commits)
```

---

## Edge Cases

- **Empty DB / no commits in range**: all histograms all-zero, after_hours_ratio=0.0, session distribution 0 sessions. Render graceful empty messages, do not panic.
- **Single timezone offset**: always convert UTC→Local before bucketing. Tests that insert UTC timestamps directly will see the local offset of the test runner — document this or force UTC in tests by using timestamps that are already at integer UTC hours.
- **days=0 validation**: return `anyhow::bail!("--days must be >= 1")`.
- **Session < 5 min exclusion (US-007)**: sessions under 5 minutes are noise (single isolated commit). Exclude from distribution to avoid skewing median.
- **CV calculation (US-009)**: if mean gap is 0 (all commits at same second), set cv=0.0 and pattern=Insufficient.
- **Rust 2024 unsafe**: if any test uses `std::env::set_var`, wrap in `unsafe {}` block per project rule.
- **Clippy in 2024 edition**: `let _ = expr;` for silencing unused results is fine; do not use `#[must_use]` unless adding to public API items that genuinely need it.

---

## New Module: src/rhythm.rs

Create this file to hold:
- `pub fn run_rhythm(days: u64, format: OutputFormat) -> anyhow::Result<()>` — orchestrator
- Import query fns from `crate::query`, render fns from `crate::output`

Register in `src/lib.rs`:
```rust
pub mod rhythm;
```

---

## Progress Report Format

After each story, output a one-line status:

```
[US-XXX] done — <what was added, 1 sentence>
```

If a story is skipped or blocked:

```
[US-XXX] blocked — <reason>
```

After completing all stories in a priority tier, output:

```
=== P0 complete. cargo build: OK. Tests: N passing, M failing (will fix at end). ===
```

---

## Documentation Updates
The final story (US-019) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Feature branch only.
- Never use `git add -A` or `git add .` — stage specific files by name.
- Do not push unless the user says so.
- Framing language: "mirror, not a score." No evaluative adjectives for metrics (no "healthy", "concerning", "good pattern"). Neutral pattern labels only.
- All new public types/fns in query.rs and output.rs must be `pub`.
- All new structs used in JSON output must `#[derive(serde::Serialize)]`.
- Do not add new DB migrations — this feature reads existing data only.
- Keep output.rs growing in the existing file (do not create output/rhythm_output.rs) unless it exceeds ~800 lines, in which case split and note it.
