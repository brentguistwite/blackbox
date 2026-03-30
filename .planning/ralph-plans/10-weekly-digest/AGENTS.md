# Agent Instructions: Weekly Digest Feature

## Project Context

Blackbox: passive git activity tracker, Rust 2024, SQLite, daemon-based.
Crate name: `blackbox-cli`. Binary: `blackbox`. Lib: `blackbox`.

Key existing patterns this feature extends:
- `query_activity(conn, from, to, session_gap_minutes, first_commit_minutes)` → `Vec<RepoSummary>`
- `ActivitySummary { period_label, total_commits, total_reviews, total_repos, total_estimated_time, total_ai_session_time, repos }`
- `run_query()` helper in main.rs — follow same pattern for digest
- `OutputFormat` enum (Pretty/Json/Csv) in output.rs — reuse as-is
- Config: `#[serde(default)]` on all new fields, `Config::default()` must stay valid

## Task Loop (9 steps — repeat per story)

1. Read the PRD story (`prd.json`) to understand what to build
2. Read relevant source files before writing any code
3. Write integration test(s) first (TDD) — `tests/digest.rs`
4. Implement the minimal code to satisfy acceptance criteria
5. Run `cargo build` — fix all errors before proceeding
6. Run `cargo test` — fix failures
7. Run `cargo clippy` — fix warnings
8. Confirm acceptance criteria met against prd.json
9. Move to next story

## Story Execution Order

US-2 → US-1 → US-3 → US-4 → US-5 → US-6 → US-7 → US-8 → US-9

Rationale: config change (US-2) unblocks query range fn (US-1); data model (US-3) unblocks formatters (US-4/5); CLI (US-6) requires both; file output (US-7) and notify (US-8) are addons; tests (US-9) formalize throughout.

## Implementation Notes

### query.rs — new functions needed

```rust
// New range function supporting week offset and configurable start day
pub fn digest_week_range(offset: i32, week_start: Weekday) -> (DateTime<Utc>, DateTime<Utc>)
```

- `offset=0` → current week, `offset=-1` → last week, etc.
- `week_start`: `chrono::Weekday::Mon` (default) or `chrono::Weekday::Sun`
- Existing `week_range()` must NOT be changed (it is used by `blackbox week`)
- Week end = start + 7 days (not `now` — full week boundary, even for current week)
- For current week use `now` as upper bound to avoid querying future

### config.rs — new field

```rust
#[serde(default)]
pub week_start_day: Option<String>,  // "monday" | "sunday", default None = monday
```

Add helper:
```rust
impl Config {
    pub fn week_start_weekday(&self) -> chrono::Weekday {
        match self.week_start_day.as_deref() {
            Some("sunday") => chrono::Weekday::Sun,
            _ => chrono::Weekday::Mon,
        }
    }
}
```

### output.rs — new structs and functions

New type `WeeklyDigest`:
```rust
pub struct WeeklyDigest {
    pub current: ActivitySummary,
    pub previous: Option<ActivitySummary>,
    pub week_start: DateTime<Utc>,
    pub week_end: DateTime<Utc>,
}
```

New functions:
- `render_digest(digest: &WeeklyDigest) -> ()` — pretty to stdout
- `render_digest_to_string(digest: &WeeklyDigest) -> String` — for tests
- `render_digest_json(digest: &WeeklyDigest) -> String`
- `render_digest_csv(digest: &WeeklyDigest) -> String`

Pretty format structure:
```
=== Weekly Digest — Mar 24–30, 2025 ===

5h 20m  |  23 commits  |  3 repos  |  2 reviews  |  AI: 1h 10m

Mon  Mar 24   8 commits   ~1h 45m
Tue  Mar 25   5 commits   ~1h 00m
Wed  Mar 26   —
Thu  Mar 27   6 commits   ~1h 20m
Fri  Mar 28   4 commits   ~0h 55m
Sat  Mar 29   —
Sun  Mar 30   —

--- Repos ---
[same as existing render_summary_to_string per-repo block]

--- vs Last Week ---
Commits:  +5  (23 vs 18)
Time:    +45m  (~5h 20m vs ~4h 35m)
Reviews:  same (2)
```

Daily breakdown requires grouping events by local date — query daily sub-ranges or group in-memory from `ActivitySummary.repos[*].events[*].timestamp`.

### main.rs — dispatch pattern

Follow the existing `run_query` helper pattern. Create `run_digest`:

```rust
fn run_digest(
    week_offset: i32,
    format: OutputFormat,
    output_file: Option<PathBuf>,
    summarize: bool,
    compare: bool,
) -> anyhow::Result<()>
```

Load config, open DB, call `digest_week_range`, call `query_activity` for current (and previous if compare=true), build `WeeklyDigest`, dispatch to renderer, handle `--output-file`.

### cli.rs — Digest variant

```rust
/// Show structured weekly digest
Digest {
    /// Output format: pretty, json, csv
    #[arg(long, default_value = "pretty")]
    format: OutputFormat,
    /// Week offset: 0=current, -1=last week
    #[arg(long, default_value = "0")]
    week: i32,
    /// Write output to file instead of stdout
    #[arg(long)]
    output_file: Option<std::path::PathBuf>,
    /// Include week-over-week comparison (pretty default: true)
    #[arg(long, default_value = "true")]
    compare: bool,
    /// Summarize using LLM
    #[arg(long)]
    summarize: bool,
    /// Send OS notification with summary
    #[arg(long)]
    notify: bool,
},
```

## Quality Checks

Run after every story implementation:

```bash
cargo build                          # must pass with 0 errors
cargo test                           # must pass
cargo clippy -- -D warnings          # must pass
```

## Documentation Updates
The final story (US-10) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## Edge Cases to Handle

| Case | Expected behavior |
|------|-------------------|
| Week with zero activity | Print "No activity for week of <date>." — don't panic |
| Partial current week (Mon-today) | Use `now` as upper bound, not end-of-week |
| `--week -1` on Monday morning | Previous week = last Mon-Sun, no partial |
| Sunday week start, mid-week query | week_start = last Sunday, end = this Saturday |
| `--output-file` path with missing parent dirs | create_dir_all, then write |
| Previous week data absent (new install) | Omit WoW section entirely, no error |
| `total_estimated_time = 0` but repos non-empty | Show "~0m", don't hide repos |
| Large offset (e.g. --week -52) | Valid — returns empty ActivitySummary if no data |
| Both `--summarize` and `--output-file` | LLM output goes to file, not stdout |

## Testing Patterns

Follow `tests/` conventions:
- `tempfile::TempDir` for DB isolation
- `git2::Repository::init()` for fake repos (not mkdir)
- `assert_cmd::Command::cargo_bin("blackbox")` for CLI integration tests
- Rust 2024: wrap `std::env::set_var` in `unsafe {}`
- Seed DB with `blackbox::db::insert_*` functions for test data
- Test date ranges: use fixed `DateTime<Utc>` values, not `Utc::now()`, for determinism

## File Map

| Story | Primary files |
|-------|--------------|
| US-2 (config) | `src/config.rs` |
| US-1 (range fn) | `src/query.rs` |
| US-3 (WoW struct) | `src/query.rs`, `src/output.rs` |
| US-4 (pretty) | `src/output.rs` |
| US-5 (json/csv) | `src/output.rs` |
| US-6 (CLI) | `src/cli.rs`, `src/main.rs` |
| US-7 (output-file) | `src/main.rs` |
| US-8 (notify) | `src/main.rs` |
| US-9 (tests) | `tests/digest.rs` |

## Branch

`feature/weekly-digest` — never commit to `main` directly.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.
