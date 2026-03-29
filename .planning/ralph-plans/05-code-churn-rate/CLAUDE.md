# Ralph Agent Instructions — code-churn-rate

You are an autonomous coding agent implementing the `code-churn-rate` feature for the blackbox CLI. Work story-by-story, write tests first (TDD), verify each story before moving to the next.

---

## Your Task

Follow these steps for each story:

1. **Read** the story from `prd.json` and identify acceptance criteria.
2. **Write tests first** — add failing tests before touching implementation.
3. **Implement** the minimal change to make tests pass.
4. **Run quality checks** (see below) — fix all failures before proceeding.
5. **Verify** acceptance criteria are met.
6. **Report** status (see Progress Report Format).
7. Repeat for the next story.

Never implement multiple stories simultaneously. Never skip to implementation before tests exist.

---

## Story Dependencies

```
US-001 (DB: commit_line_stats)
  └─ US-003 (insert/query helpers)
       └─ US-006 (backfill on poll)
            └─ US-012 (e2e integration test)
       └─ US-007 (churn algorithm)
            └─ US-010 (pretty output)
                 └─ US-011 (json/csv output)
            └─ US-009 (CLI command)
                 └─ US-012

US-002 (DB: churn_events)
  └─ US-004 (insert_churn_event)
       └─ US-007

US-005 (diff stats — no deps)
  └─ US-006

US-008 (config field — no deps)
  └─ US-009
```

Implement in order: US-001 → US-002 → US-003 → US-004 → US-005 → US-006 → US-007 → US-008 → US-009 → US-010 → US-011 → US-012.

---

## Quality Checks

Run after every story. All must pass before marking done.

```bash
cargo build 2>&1
cargo test  2>&1
cargo clippy -- -D warnings 2>&1
```

Fix all errors and warnings before proceeding.

---

## Project Context

**Crate:** `blackbox-cli` (lib name `blackbox`, binary `blackbox`). Edition 2024. Single crate, SQLite-backed.

**Key files:**
- `src/db.rs` — migrations array in `open_db()`, insert/query helpers
- `src/git_ops.rs` — `poll_repo()`, `is_own_commit()`, `get_user_identity()`
- `src/churn.rs` — NEW file for this feature (diff stats + churn algorithm)
- `src/cli.rs` — `Commands` enum (clap derive)
- `src/main.rs` — match arm dispatch
- `src/output.rs` — render functions
- `src/config.rs` — `Config` struct with `#[serde(default)]` fields
- `src/lib.rs` — module declarations (must add `pub mod churn;`)
- `tests/churn_test.rs` — NEW integration test file

**Existing DB schema (migrations 1-6):**
```
git_activity(id, repo_path, event_type, branch, commit_hash, author, message, timestamp, created_at, source_branch)
  index: (repo_path, timestamp), unique partial: (repo_path, commit_hash) WHERE commit_hash NOT NULL
directory_presence(id, repo_path, entered_at, left_at)
review_activity(id, repo_path, pr_number, pr_title, pr_url, review_action, reviewed_at, created_at)
  unique: (repo_path, pr_number, reviewed_at)
ai_sessions(id, repo_path, session_id, started_at, ended_at, turns, created_at)
  unique: session_id
```

New tables added by this feature (migrations 7 and 8):
```
commit_line_stats(id, repo_path, commit_hash, file_path, lines_added, lines_deleted, committed_at, created_at)
  unique: (repo_path, commit_hash, file_path)
churn_events(id, repo_path, original_commit_hash, churn_commit_hash, file_path, lines_churned, churn_window_days, detected_at)
  unique: (repo_path, original_commit_hash, churn_commit_hash, file_path)
```

**git2 diff API (for US-005):**

```rust
use git2::{Repository, Commit, DiffOptions, DiffFormat, Delta};

// Get diff between commit and its parent (or empty tree for initial commit)
let tree = commit.tree()?;
let parent_tree = if commit.parent_count() > 0 {
    Some(commit.parent(0)?.tree()?)
} else {
    None
};
let mut opts = DiffOptions::new();
let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;

// Walk the diff to count lines
diff.foreach(
    &mut |_delta, _progress| true,   // file_cb
    None,                              // binary_cb
    None,                              // hunk_cb
    Some(&mut |delta, _hunk, line| {  // line_cb
        match line.origin() {
            '+' => { /* lines_added += 1 */ }
            '-' => { /* lines_deleted += 1 */ }
            _ => {}
        }
        true
    }),
)?;
```

Access file path per delta: `delta.new_file().path()` (UTF-8 lossy). For renames, `delta.status() == Delta::Renamed`: emit old path (deleted lines) + new path (added lines).

Binary files: the binary_cb is None; git2 will not invoke line_cb for binary hunks, so binary files are automatically skipped.

**poll_repo structure (relevant to US-006):**

`poll_repo` in `src/git_ops.rs` has two commit-processing paths:
1. First-poll backfill: walks revwalk from HEAD, stops at midnight or 50 commits
2. Incremental: walks revwalk from `push_head`, hidden at `last_oid`

In both paths, after calling `db::insert_activity(...)`, add:
```rust
if let Ok(file_stats) = crate::churn::diff_commit_stats(&repo, &commit) {
    let ts_str = ts.as_str(); // RFC3339 string already computed
    for stat in &file_stats {
        if let Err(e) = db::insert_commit_line_stats(conn, db_repo_path, &hash, &stat.file_path, stat.lines_added as i64, stat.lines_deleted as i64, ts_str) {
            log::warn!("insert_commit_line_stats failed: {e}");
        }
    }
}
```

Only call for non-merge commits (diff_commit_stats returns empty for merges, but guard with `commit.parent_count() <= 1` to avoid unnecessary work).

**Churn algorithm detail (for US-007):**

```
Input: all commit_line_stats for repo_path within last window_days
Group rows by file_path → Map<file_path, Vec<(commit_hash, lines_added, lines_deleted, committed_at)>>

For each file:
  Sort entries by committed_at ASC
  For each pair (A, B) where A.committed_at < B.committed_at:
    If (B.committed_at - A.committed_at) <= window_days:
      churned = min(A.lines_added, B.lines_deleted)
      Insert churn_event(original=A.hash, churn=B.hash, file, churned, window)
      total_churned += churned

total_written = sum of all lines_added across all stats
churn_rate = if total_written == 0 { 0.0 } else { total_churned as f64 / total_written as f64 * 100.0 }
```

Note: this is a heuristic. It does not do true line-level blame tracking. It approximates "how many of A's added lines did B delete" as `min(A.lines_added, B.lines_deleted)`. This is the same approximation GitClear uses in their published methodology.

**Config pattern (for US-008):**

```rust
// In Config struct:
#[serde(default = "default_churn_window_days")]
pub churn_window_days: u32,

// Outside struct:
fn default_churn_window_days() -> u32 { 14 }
```

**CLI pattern (for US-009):**

```rust
// In cli.rs Commands enum:
/// Show code churn rate for tracked repos
Churn {
    /// Time window in days to detect churn (default: from config)
    #[arg(long)]
    window: Option<u32>,
    /// Filter to a specific repo path
    #[arg(long)]
    repo: Option<String>,
    /// Output format: pretty, json, csv
    #[arg(long, default_value = "pretty")]
    format: OutputFormat,
},
```

In `is_exempt_from_config_check`: `Churn` is NOT exempt (requires config).

**Adding churn module to lib.rs:** add `pub mod churn;` alongside existing module declarations.

---

## Edge Cases

- **Repo with no commit_line_stats:** `compute_churn` returns `ChurnReport { total_lines_written: 0, churned_lines: 0, churn_rate_pct: 0.0, ... }`. Never divide by zero.
- **Binary files:** git2 line_cb not called for binary hunks → automatically excluded from stats.
- **Rename vs edit:** treat as two separate file paths. Old path gets lines_deleted entry, new path gets lines_added entry. They do not cross-contribute to churn (different file_path keys).
- **Merge commits:** `diff_commit_stats` returns `Vec::new()` when `commit.parent_count() > 1`. Already checked before calling in poll_repo.
- **Initial commit (no parent):** diff against empty tree using `repo.diff_tree_to_tree(None, Some(&tree), opts)`.
- **Shallow clones:** revwalk may hit a missing OID boundary. Existing `break` on `Err` in poll_repo handles this; diff on those commits is never attempted.
- **Window = 0:** no pair (A, B) satisfies `duration <= 0 days` → churn_rate_pct = 0.0.
- **Same commit hash appearing twice in stats:** INSERT OR IGNORE on (repo_path, commit_hash, file_path) prevents duplicates at DB level.
- **Very large repos:** commit_line_stats query is bounded by `committed_at >= now - window_days`. Churn computation is O(files × commits_per_file²) — acceptable for typical windows of 7-30 days.
- **Repos that no longer exist on disk:** `compute_churn` only reads from DB (no filesystem access) — works fine.

---

## Testing Patterns

- Use `tempfile::NamedTempFile` for DB, `tempfile::TempDir` for git repos
- Create test repos: `git2::Repository::init(dir.path())?`
- Create test commits programmatically with git2:
  ```rust
  fn make_commit(repo: &Repository, message: &str, files: &[(&str, &str)]) -> git2::Oid {
      let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
      let mut index = repo.index().unwrap();
      for (name, content) in files {
          std::fs::write(repo.workdir().unwrap().join(name), content).unwrap();
          index.add_path(std::path::Path::new(name)).unwrap();
      }
      index.write().unwrap();
      let tree_oid = index.write_tree().unwrap();
      let tree = repo.find_tree(tree_oid).unwrap();
      let parents: Vec<git2::Commit> = repo.head().ok()
          .and_then(|h| h.peel_to_commit().ok())
          .map(|c| vec![c])
          .unwrap_or_default();
      let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
      repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs).unwrap()
  }
  ```
- Rust 2024: `unsafe { std::env::set_var(...) }` required if needed in tests
- Test file: `tests/churn_test.rs`; DB tests in `tests/db_test.rs` if it exists, else add to churn_test.rs

---

## Progress Report Format

After each story, output:

```
Story: US-XXX — <title>
Status: DONE | BLOCKED
Tests added: <list of new test function names>
Files changed: <list of files>
Notes: <anything surprising or deferred>
```

---

## Documentation Updates
The final story (US-013) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Branch is `feature/code-churn-rate`.
- Never skip a story or reorder without noting a blocker.
- `src/lib.rs` must declare `pub mod churn;` before any code in churn.rs is reachable.
- All DB helpers use `INSERT OR IGNORE` + unique indexes — never `INSERT OR REPLACE` (would change row id).
- `diff_commit_stats` must not panic on any repo state; wrap git2 errors with `?` and let callers log+continue.
- When adding fields to structs used in tests, search all test files for construction sites (compiler will flag them).
- Edition 2024: `unsafe` block required for `std::env::set_var` in tests.
- The `churn_rate_pct` calculation must guard against `total_lines_written == 0` to avoid NaN/inf.
