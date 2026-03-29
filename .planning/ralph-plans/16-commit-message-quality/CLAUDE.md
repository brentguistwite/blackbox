# Agent Instructions: commit-message-quality

## Project context
Blackbox (crate: `blackbox-cli`, binary: `blackbox`). Rust edition 2024. SQLite via `rusqlite 0.38` + `rusqlite_migration`. Commits stored in `git_activity` table (`event_type IN ('commit','branch_switch','merge')`). No new deps needed — scoring is pure Rust string logic.

Branch: `feature/commit-message-quality`

## Task loop (9 steps, repeat until done)

1. **Read the PRD** — load `.planning/ralph-plans/16-commit-message-quality/prd.json`. Work stories in dependency order: US-016-01 → 02 → 03 → 04 → 05 → 06 → 07 → 08.
2. **Read relevant source** before touching a file. Minimum reads per story:
   - DB work → `src/db.rs` (migrations array, insert pattern)
   - Query work → `src/query.rs` (BTreeMap grouping, DateTime<Utc> handling)
   - CLI work → `src/cli.rs` (Commands enum), `src/main.rs` (match arm pattern)
   - Output work → `src/output.rs` (render_summary_to_string pattern, colored crate usage)
   - Tests → `tests/` (existing test file for patterns)
3. **Implement** the story. Follow existing patterns exactly.
4. **Build** — `cargo build 2>&1 | head -40`. Fix all errors before proceeding.
5. **Clippy** — `cargo clippy -- -D warnings 2>&1 | head -40`. Fix all warnings.
6. **Test** — `cargo test 2>&1 | tail -30`. All tests must pass.
7. **Mark story done** in working notes.
8. **Next story** — return to step 2.
9. **Final check** — `cargo build && cargo test && cargo clippy -- -D warnings`. All green.

## File map (which file owns what)

| Work | File |
|------|------|
| score_message(), is_vague() | `src/commit_quality.rs` (new module) |
| DB migration + insert_commit_quality() | `src/db.rs` |
| score_and_store() call in poll | `src/git_ops.rs` |
| commit_quality_trend(), find_reverted_commits() | `src/query.rs` |
| Commands::CommitQuality | `src/cli.rs` |
| match arm + run logic | `src/main.rs` |
| render_commit_quality() | `src/output.rs` |
| module declaration | `src/lib.rs` |
| all tests | `tests/commit_quality.rs` |

## Key implementation patterns

### New module
Add `pub mod commit_quality;` to `src/lib.rs`. New file `src/commit_quality.rs`.

### DB migration (US-016-03)
Append to the `vec![]` in `open_db()` in `src/db.rs`:
```rust
M::up(
    "CREATE TABLE IF NOT EXISTS commit_quality (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        repo_path TEXT NOT NULL,
        commit_hash TEXT NOT NULL,
        score INTEGER NOT NULL,
        is_vague INTEGER NOT NULL DEFAULT 0,
        scored_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_commit_quality_repo_hash
        ON commit_quality(repo_path, commit_hash);"
),
```

### insert_commit_quality pattern (matches insert_review)
```rust
pub fn insert_commit_quality(
    conn: &Connection,
    repo_path: &str,
    commit_hash: &str,
    score: u8,
    is_vague: bool,
) -> anyhow::Result<bool> {
    match conn.execute(
        "INSERT OR IGNORE INTO commit_quality (repo_path, commit_hash, score, is_vague)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![repo_path, commit_hash, score as i64, is_vague as i64],
    ) {
        Ok(0) => Ok(false),
        Ok(_) => Ok(true),
        Err(e) => Err(e.into()),
    }
}
```

### Calling score_and_store from git_ops.rs
After each `db::insert_activity(...)` call that returns `Ok(true)` for a `commit` event, call:
```rust
let score = crate::commit_quality::score_message(&message);
let vague = crate::commit_quality::is_vague(&message);
let _ = db::insert_commit_quality(conn, db_repo_path, &hash, score, vague);
```
Do this in both the backfill path and the incremental path.

### WeeklyQuality struct (query.rs)
```rust
#[derive(Debug, Clone)]
pub struct WeeklyQuality {
    pub week_start: chrono::NaiveDate,
    pub commit_count: usize,
    pub avg_score: f64,
    pub vague_count: usize,
    pub vague_pct: f64,
}
```

### commit_quality_trend SQL
```sql
SELECT
    cq.score,
    cq.is_vague,
    ga.timestamp
FROM commit_quality cq
JOIN git_activity ga ON ga.repo_path = cq.repo_path AND ga.commit_hash = cq.commit_hash
WHERE ga.timestamp >= ?1
ORDER BY ga.timestamp ASC
```
Group rows into ISO weeks in Rust (not SQL) using `NaiveDate::from_isoywd_opt`.

### CLI (cli.rs)
```rust
/// Show commit message quality scores and trends
CommitQuality {
    /// Number of weeks to include in trend (1–52)
    #[arg(long, default_value = "8", value_parser = clap::value_parser!(u32).range(1..=52))]
    weeks: u32,
    /// Output format: pretty, json, csv
    #[arg(long, default_value = "pretty")]
    format: OutputFormat,
    /// Include revert correlation analysis
    #[arg(long)]
    show_reverts: bool,
},
```

### main.rs match arm
```rust
Commands::CommitQuality { weeks, format, show_reverts } => {
    let config = blackbox::config::load_config()?;
    let data_dir = blackbox::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = blackbox::db::open_db(&db_path)?;
    let trend = blackbox::query::commit_quality_trend(&conn, weeks)?;
    let reverts = if show_reverts {
        blackbox::query::find_reverted_commits(&conn)?
    } else {
        vec![]
    };
    let out = blackbox::output::render_commit_quality(&trend, &reverts, &format);
    println!("{}", out);
}
```

## Scoring algorithm (US-016-01)

```
base = 50
+ up to +30 for subject length 10–72 chars (proportional)
+ 20 if conventional commit prefix (type(scope): or type: where type in feat|fix|chore|docs|refactor|test|style|perf|ci|build|revert)
+ 10 if body present (blank line after subject)
- 20 if subject < 10 chars
- 10 if subject > 72 chars
- 5 if all lowercase with no punctuation in subject
clamp to 0–100
```

Merge commits: return 50 immediately if `msg.starts_with("Merge ")`.

## Vague patterns list (US-016-02)

Case-insensitive, whole-message trim match OR message word-count ≤ 3 containing only these tokens:
`wip`, `fix`, `fixes`, `fixed`, `update`, `updates`, `updated`, `misc`, `stuff`, `changes`,
`cleanup`, `clean up`, `refactor`, `test`, `tests`, `temp`, `tmp`, `asdf`, `...`, `.`, `!!`

Non-ASCII majority check: if `msg.chars().filter(|c| !c.is_ascii()).count() * 2 > msg.len()` → not vague.

## Edge cases

| Case | Handling |
|------|----------|
| Empty message | score=0, is_vague=true |
| Merge commit (`parent_count > 1`) | score=50, is_vague=false, do NOT call score_and_store |
| Branch switch event | No quality scoring — only `commit` events |
| Non-English message | Non-ASCII majority → is_vague=false |
| Conventional commit with long body | Score ≥ 80 expected |
| Message with only whitespace | Trim first, treat as empty |
| Revert commit (`Revert "..."`) | score=50 (merge-like exemption), is_vague=false |
| `commit_hash IS NULL` in git_activity | Skip — no hash to key on |
| Backfill cap | Max 200 rows in backfill path to avoid slow startup |

## Testing patterns (Rust 2024)

```rust
// unsafe required for set_var in Rust 2024
unsafe { std::env::set_var("BLACKBOX_DATA_DIR", tmp.path()); }

// tempfile pattern
let tmp = tempfile::tempdir().unwrap();
let db_path = tmp.path().join("test.db");
let conn = blackbox::db::open_db(&db_path).unwrap();
```

Test file: `tests/commit_quality.rs`
Import: `use blackbox::commit_quality::{score_message, is_vague};`

## Documentation Updates
The final story requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## Quality gates (must all pass before PR)

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

No `#[allow(...)]` suppressions unless pre-existing in codebase.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.
