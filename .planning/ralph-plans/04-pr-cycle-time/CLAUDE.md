# Ralph Agent Instructions — pr-cycle-time

You are an autonomous coding agent implementing the `pr-cycle-time` feature for the `blackbox` CLI. Read prd.json in this directory for the full story list. Implement stories in dependency order. Do not commit to `main` — always work on `feature/pr-cycle-time`.

---

## Your Task

Follow these steps in order:

1. **Read prd.json** — understand all stories, priorities, and deps before writing any code.
2. **Create feature branch**: `git checkout -b feature/pr-cycle-time`
3. **Implement P0 stories first** (US-001 through US-012), then P1 (US-013 through US-015), then P2 (US-016). Within each priority tier, respect deps order.
4. **Within each story**: write the test first (TDD), then implement until the test passes.
5. **After each story**: `cargo build` must pass before moving to the next story.
6. **After all stories**: run `cargo test` and `cargo clippy` and fix all failures/warnings.
7. **Do not fix tests after every iteration** during rapid implementation — batch test fixes at the end of each priority tier, or when explicitly asked.
8. **Write a progress report** after completing each story (see format below).
9. **Do not push** unless the user asks.

---

## Story Dependencies

```
US-001 (DB migration: pr_snapshots)
  ├─ US-002 (gh fetch: GhPrDetail)
  │    └─ US-003 (db: upsert_pr_snapshot)
  │         └─ US-004 (collect_pr_snapshots)
  │              └─ US-005 (integrate into full_scan)
  │                   └─ US-013 (remove live enrich_with_prs)
  └─ US-006 (query_pr_cycle_stats)
       ├─ US-007 (render_pr_cycle_stats)
       │    └─ US-008 (CLI: blackbox prs)
       │         └─ US-012 (CLI integration test)
       ├─ US-009 (JSON output)
       ├─ US-011 (query integration test)
       └─ US-014 (edge case: closed-without-merge)

US-003 → US-010 (DB migration + upsert integration test)
US-004 → US-015 (edge case: no PRs / gh not installed)
US-002 → US-016 (edge case: rate limit / limit 50)
```

---

## Quality Checks

Before declaring done, all three must pass cleanly:

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

Fix all clippy warnings. Do not use `#[allow(clippy::...)]` unless genuinely inapplicable and explained.

---

## Project Context

### Key files to read before writing code

- `src/enrichment.rs` — OnceLock pattern for gh_available(), existing fetch_prs/fetch_reviewed_prs/collect_reviews. New fetch_pr_details() and collect_pr_snapshots() go here.
- `src/db.rs` — Migration array pattern (rusqlite_migration). New migration = new M::up() at end of vec. New upsert_pr_snapshot() fn goes here.
- `src/query.rs` — All DB query fns. Add PrCycleStats, PrMetrics, query_pr_cycle_stats() here.
- `src/output.rs` — All render fns. Add render_pr_cycle_stats(), render_pr_cycle_json() here.
- `src/poller.rs` — full_scan() fn. Add collect_pr_snapshots call here.
- `src/cli.rs` — Commands enum. Add Prs variant here.
- `src/main.rs` — match arm dispatch. Add Commands::Prs arm here.
- `src/lib.rs` — Module declarations. If adding a new module, register here.
- `tests/query_test.rs` — setup_db() helper pattern to copy/reference for new test file.

### Existing gh CLI integration patterns (enrichment.rs)

**OnceLock for availability check (copy this pattern):**
```rust
fn gh_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("which").arg("gh").output()
            .map(|o| o.status.success()).unwrap_or(false)
    })
}
```

**Subprocess with thread-based timeout (no recv_timeout — copy exactly):**
```rust
fn fetch_pr_details(repo_path: &str) -> Option<Vec<GhPrDetail>> {
    let child = Command::new("gh")
        .args([
            "pr", "list",
            "--state", "all",
            "--limit", "50",
            "--json", "number,title,url,state,headRefName,baseRefName,author,createdAt,mergedAt,closedAt,reviews,additions,deletions,commits",
        ])
        .current_dir(repo_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let handle = std::thread::spawn(move || child.wait_with_output());
    match handle.join() {
        Ok(Ok(output)) if output.status.success() => {
            serde_json::from_slice(&output.stdout).ok()
        }
        _ => None,
    }
}
```

**Worktree dedup pattern (copy from collect_reviews):**
```rust
let mut seen_main_repos: HashSet<std::path::PathBuf> = HashSet::new();
for repo_path in repo_paths {
    let main_repo = if repo_scanner::is_worktree(repo_path).is_some() {
        repo_scanner::resolve_main_repo(repo_path).unwrap_or_else(|_| repo_path.clone())
    } else {
        repo_path.clone()
    };
    if !seen_main_repos.insert(main_repo.clone()) {
        continue;
    }
    // ... fetch and upsert
}
```

### DB migration pattern (db.rs)

Migrations are an ordered vec of M::up() calls. Always append a new entry — never edit existing entries:

```rust
let migrations = Migrations::new(vec![
    M::up("CREATE TABLE IF NOT EXISTS git_activity ..."),  // migration 1
    M::up("ALTER TABLE git_activity ADD COLUMN source_branch TEXT;"),  // migration 2
    // ... existing migrations ...
    // NEW — migration N:
    M::up("CREATE TABLE IF NOT EXISTS pr_snapshots (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        repo_path TEXT NOT NULL,
        pr_number INTEGER NOT NULL,
        title TEXT NOT NULL,
        url TEXT NOT NULL,
        state TEXT NOT NULL,
        head_ref TEXT NOT NULL,
        base_ref TEXT NOT NULL,
        author_login TEXT,
        created_at_gh TEXT,
        merged_at TEXT,
        closed_at TEXT,
        first_review_at TEXT,
        additions INTEGER,
        deletions INTEGER,
        commits INTEGER,
        iteration_count INTEGER,
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_pr_snapshots_repo_pr ON pr_snapshots(repo_path, pr_number);
    CREATE INDEX IF NOT EXISTS idx_pr_snapshots_repo_state ON pr_snapshots(repo_path, state);"),
]);
```

### upsert_pr_snapshot pattern

```rust
pub fn upsert_pr_snapshot(
    conn: &Connection,
    repo_path: &str,
    pr: &GhPrDetail,
) -> anyhow::Result<()> {
    // Compute first_review_at: earliest submitted_at where state != "PENDING"
    let first_review_at: Option<String> = pr.reviews.iter()
        .filter(|r| r.state != "PENDING")
        .map(|r| r.submitted_at.clone())
        .min();

    // iteration_count: number of CHANGES_REQUESTED reviews
    let iteration_count: i64 = pr.reviews.iter()
        .filter(|r| r.state == "CHANGES_REQUESTED")
        .count() as i64;

    conn.execute(
        "INSERT OR REPLACE INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref,
          author_login, created_at_gh, merged_at, closed_at, first_review_at,
          additions, deletions, commits, iteration_count)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        rusqlite::params![
            repo_path,
            pr.number as i64,
            &pr.title,
            &pr.url,
            &pr.state,
            &pr.head_ref_name,
            &pr.base_ref_name,
            pr.author.as_ref().map(|a| &a.login),
            &pr.created_at,
            &pr.merged_at,
            &pr.closed_at,
            first_review_at,
            pr.additions,
            pr.deletions,
            pr.commits.len() as i64,
            iteration_count,
        ],
    )?;
    Ok(())
}
```

### GhPrDetail struct (new, in enrichment.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhCommit {
    pub oid: String,
    #[serde(rename = "committedDate")]
    pub committed_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhPrDetail {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    #[serde(rename = "headRefName")]
    pub head_ref_name: String,
    #[serde(rename = "baseRefName")]
    pub base_ref_name: String,
    #[serde(default)]
    pub author: Option<GhReviewAuthor>,  // reuse existing type
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(rename = "mergedAt")]
    pub merged_at: Option<String>,
    #[serde(rename = "closedAt")]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub reviews: Vec<GhReview>,          // reuse existing type
    #[serde(default)]
    pub additions: Option<i64>,
    #[serde(default)]
    pub deletions: Option<i64>,
    #[serde(default)]
    pub commits: Vec<GhCommit>,
}
```

### query_pr_cycle_stats signature

```rust
pub fn query_pr_cycle_stats(
    conn: &Connection,
    repo_path_filter: Option<&str>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<PrCycleStats>
```

Filter on `created_at_gh >= from AND created_at_gh <= to`. When `repo_path_filter` is Some, add `AND repo_path = ?`.

### Median computation helper

```rust
fn median_f64(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() { return None; }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        Some((values[mid - 1] + values[mid]) / 2.0)
    } else {
        Some(values[mid])
    }
}
```

### CLI variant pattern (cli.rs)

```rust
/// Show PR cycle time metrics
Prs {
    /// Number of days to analyze (default 30)
    #[arg(long, default_value_t = 30)]
    days: u64,
    /// Filter to a specific repo path
    #[arg(long)]
    repo: Option<String>,
    /// Output format: pretty, json
    #[arg(long, default_value = "pretty")]
    format: OutputFormat,
},
```

### main.rs dispatch pattern

```rust
Commands::Prs { days, repo, format } => {
    if days == 0 {
        anyhow::bail!("--days must be >= 1");
    }
    let config = blackbox::config::load_config()?;
    let data_dir = blackbox::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = blackbox::db::open_db(&db_path)?;
    let to = chrono::Utc::now();
    let from = to - chrono::Duration::days(days as i64);
    let stats = blackbox::query::query_pr_cycle_stats(&conn, repo.as_deref(), from, to)?;
    match format {
        OutputFormat::Pretty => println!("{}", blackbox::output::render_pr_cycle_stats(&stats)),
        OutputFormat::Json => println!("{}", blackbox::output::render_pr_cycle_json(&stats)),
        OutputFormat::Csv => anyhow::bail!("--format csv not supported for prs command"),
    }
}
```

### Test file convention

Create `tests/pr_cycle_test.rs`. Copy the `setup_db()` helper from `tests/query_test.rs`:

```rust
use blackbox::db;
use rusqlite::Connection;
use tempfile::NamedTempFile;

fn setup_db() -> (Connection, NamedTempFile) {
    let f = NamedTempFile::new().unwrap();
    let conn = db::open_db(f.path()).unwrap();
    (conn, f)
}
```

Insert test rows directly via `conn.execute("INSERT INTO pr_snapshots ...")` for query tests.

---

## Edge Cases

- **gh not installed**: `gh_available()` returns false → `collect_pr_snapshots` logs debug and returns immediately. No warn. `blackbox prs` still works (shows empty data from DB).
- **Not a GitHub repo**: `fetch_pr_details` returns None silently (gh exits non-zero). Skip that repo without error.
- **gh authentication expired**: same as above — non-zero exit → None → skip.
- **Rate limiting**: gh handles rate limits internally. Non-zero exit → None → skip. No retry in blackbox.
- **Repo with 0 open PRs**: `fetch_pr_details` returns `Some([])` — no upserts, no error. `blackbox prs` shows "No PR data available for this period".
- **Closed-without-merge PRs**: `merged_at = NULL` in DB. `cycle_time_hours = None`. Display state as 'closed'. Do not count in `merged_prs`.
- **PRs with 0 reviews**: `first_review_at = NULL`. `time_to_first_review_hours = None`. `iteration_count = 0`.
- **PRs with no additions/deletions field** (gh API quirk on some repos): both NULL → `size_lines = None`.
- **DateTime parse failures** from gh API: treat as None for that field — do not propagate error.
- **INSERT OR REPLACE semantics**: replaces entire row including `updated_at`. The unique index on `(repo_path, pr_number)` is what triggers the replace. This means re-polling updates PR state (e.g. open → merged) correctly.
- **Worktree paths**: always resolve to main repo path before fetching PRs (same as reviews). Prevents duplicate gh calls and stores under canonical path.
- **Rust 2024 unsafe**: any test using `std::env::set_var` must wrap in `unsafe {}`.
- **Clippy in 2024 edition**: `if let Ok(_) = x` where result is unused → use `x.ok();` or `let _ = x;`.

---

## Progress Report Format

After each story, output a one-line status:

```
[US-XXX] done — <what was added, 1 sentence>
```

If blocked:

```
[US-XXX] blocked — <reason>
```

After completing all stories in a priority tier:

```
=== P0 complete. cargo build: OK. Tests: N passing, M failing (will fix at end). ===
```

---

## Documentation Updates
The final story (US-017) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Feature branch only.
- Never use `git add -A` or `git add .` — stage specific files by name.
- Do not push unless the user says so.
- All new public types/fns must be `pub`.
- All structs used in JSON output must `#[derive(serde::Serialize)]`.
- Metric display language: neutral only. Cycle time is a measurement, not a grade.
- Keep `db.rs`, `query.rs`, `output.rs`, `enrichment.rs` as growing single files — do not split unless a file exceeds ~800 lines and you document the split.
- `GhReviewAuthor` and `GhReview` already exist in enrichment.rs — reuse them, do not duplicate.
- The `--limit 50` cap in `fetch_pr_details` is intentional — document it in a comment, do not remove it.
