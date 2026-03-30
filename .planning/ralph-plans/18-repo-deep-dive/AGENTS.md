# Ralph Agent Instructions — repo-deep-dive

You are an autonomous coding agent implementing the `repo-deep-dive` feature for the `blackbox` CLI. Read prd.json in this directory for the full story list. Implement stories in dependency order. Never commit to `main` — always work on `feature/repo-deep-dive`.

---

## Your Task

Follow these steps in order:

1. **Read prd.json** — understand all stories, priorities, deps before writing any code.
2. **Create feature branch**: `git checkout -b feature/repo-deep-dive`
3. **Implement P0 stories first** (US-001 through US-014), then P1 (US-015 and US-006 if deferred).
4. **Within each story**: write the test first (TDD), then implement until the test passes.
5. **After each story**: `cargo build` must pass before moving to the next.
6. **After all stories**: run `cargo test` and `cargo clippy` and fix all failures/warnings.
7. **Do not fix tests after every iteration** during rapid implementation — batch test fixes at the end of each priority tier, or when explicitly asked.
8. **Write a progress report** after completing each story (see format below).
9. **Do not push** unless the user asks.

---

## Story Dependencies

```
US-001 (path resolution + DB lookup)
  ├─ US-002 (all-time DB query for single repo)
  │    ├─ US-005 (time invested)
  │    ├─ US-006 (PR history — P1)
  │    └─ US-007 (branch activity)
  │         └─ US-014 (tests: top_files + branch_activity)
  ├─ US-003 (language breakdown)
  │    └─ US-013 (tests: language breakdown)
  ├─ US-004 (top files changed)
  │    └─ US-014
  └─ US-012 (tests: path resolution)

US-008 (RepoDeepDive aggregate struct) — deps: US-001..US-007
  ├─ US-009 (pretty output)
  │    └─ US-011 (CLI wiring)
  │         └─ US-015 (integration test — P1)
  └─ US-010 (JSON output)
       └─ US-011
```

Recommended implementation order:
US-001 → US-012 → US-002 → US-003 → US-013 → US-004 → US-014 → US-005 → US-007 → US-008 → US-009 → US-010 → US-011 → US-006 → US-015

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

- `src/query.rs` — `ActivityEvent`, `ReviewInfo`, `AiSessionInfo`, `RepoSummary`, `estimate_time_v2()`, `query_reviews()` pattern. Your `query_repo_all_time()` mirrors the per-repo portion of `query_activity()` but without a time window.
- `src/db.rs` — all DB functions, migrations array, `open_db()` pattern. No new migrations needed for this feature.
- `src/git_ops.rs` — how git2::Repository is opened, revwalk pattern, commit walking. Your `compute_top_files()` follows the same revwalk pattern.
- `src/enrichment.rs` — `PrInfo`, `gh_available()` OnceLock pattern, `fetch_prs()` subprocess pattern. `fetch_repo_pr_history()` reuses these patterns for `--state all`.
- `src/output.rs` — `OutputFormat` enum, `format_duration()`, `render_summary_to_string()` for colored output style. Add `render_repo_pretty()` and `render_repo_json()` here.
- `src/cli.rs` — existing Commands enum. Add `Repo` variant.
- `src/main.rs` — `run_query()` pattern for DB open + config load + dispatch. Mirror for the new `Commands::Repo` match arm.
- `src/lib.rs` — add `pub mod repo_deep_dive;` here.
- `tests/` — look at existing test files for `tempfile`, `git2::Repository::init()`, `assert_cmd` patterns.

### New module: src/repo_deep_dive.rs

Create this file from scratch. It owns all deep-dive logic: path resolution, DB queries, git2 computations, and the orchestrating `build_deep_dive()` function.

Key public API surface:

```rust
// Path resolution
pub fn resolve_repo_path(input: &str) -> anyhow::Result<std::path::PathBuf>
pub fn find_db_repo_path(conn: &rusqlite::Connection, canonical: &std::path::Path) -> anyhow::Result<Option<String>>

// DB query
pub struct RepoAllTimeData { pub repo_path: String, pub events: Vec<crate::query::ActivityEvent>, pub reviews: Vec<crate::query::ReviewInfo>, pub ai_sessions: Vec<crate::query::AiSessionInfo> }
pub fn query_repo_all_time(conn: &rusqlite::Connection, repo_path: &str) -> anyhow::Result<RepoAllTimeData>

// git2 computations
pub struct LanguageBreakdown { pub language: String, pub file_count: usize, pub line_count: usize, pub percent: f64 }
pub fn compute_language_breakdown(repo_path: &std::path::Path) -> anyhow::Result<Vec<LanguageBreakdown>>

pub struct FileChurnEntry { pub path: String, pub change_count: usize }
pub fn compute_top_files(repo_path: &std::path::Path, limit: usize) -> anyhow::Result<Vec<FileChurnEntry>>

// Derived from DB data
pub fn compute_time_invested(data: &RepoAllTimeData, session_gap_minutes: u64, first_commit_minutes: u64) -> chrono::Duration

pub struct BranchActivity { pub name: String, pub commit_count: usize, pub last_active: chrono::DateTime<chrono::Utc> }
pub fn compute_branch_activity(data: &RepoAllTimeData) -> Vec<BranchActivity>

pub struct RepoPrEntry { pub number: u64, pub title: String, pub state: String, pub branch: Option<String>, pub url: Option<String> }
pub fn fetch_repo_pr_history(repo_path: &str, data: &RepoAllTimeData) -> Vec<RepoPrEntry>

// Aggregate
pub struct RepoDeepDive { pub repo_path: String, pub repo_name: String, pub total_commits: usize, pub first_commit_at: Option<chrono::DateTime<chrono::Utc>>, pub last_commit_at: Option<chrono::DateTime<chrono::Utc>>, pub total_estimated_time: chrono::Duration, pub languages: Vec<LanguageBreakdown>, pub top_files: Vec<FileChurnEntry>, pub branches: Vec<BranchActivity>, pub prs: Vec<RepoPrEntry>, pub tracked: bool }
pub fn build_deep_dive(repo_path: &str, conn: &rusqlite::Connection, config: &crate::config::Config) -> anyhow::Result<RepoDeepDive>
```

### compute_language_breakdown — git2 tree walk pattern

```rust
pub fn compute_language_breakdown(repo_path: &Path) -> anyhow::Result<Vec<LanguageBreakdown>> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return Ok(vec![]),
    };
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(vec![]), // empty repo
    };
    let tree = match head.peel_to_tree() {
        Ok(t) => t,
        Err(_) => return Ok(vec![]),
    };

    // extension -> (file_count, line_count)
    let mut ext_counts: std::collections::HashMap<String, (usize, usize)> = Default::default();

    tree.walk(git2::TreeWalkMode::PreOrder, |_, entry| {
        if entry.kind() != Some(git2::ObjectType::Blob) {
            return git2::TreeWalkResult::Ok;
        }
        let name = match entry.name() { Some(n) => n, None => return git2::TreeWalkResult::Ok };
        let ext = match std::path::Path::new(name).extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_lowercase(),
            None => return git2::TreeWalkResult::Ok,
        };
        let blob = match repo.find_blob(entry.id()) {
            Ok(b) => b,
            Err(_) => return git2::TreeWalkResult::Ok,
        };
        if blob.is_binary() {
            return git2::TreeWalkResult::Ok;
        }
        let lines = blob.content().split(|&b| b == b'\n').count();
        let e = ext_counts.entry(ext).or_insert((0, 0));
        e.0 += 1;
        e.1 += lines;
        git2::TreeWalkResult::Ok
    })?;

    let total_lines: usize = ext_counts.values().map(|(_, l)| l).sum();
    if total_lines == 0 { return Ok(vec![]); }

    // aggregate by language
    let mut lang_map: std::collections::HashMap<String, (usize, usize)> = Default::default();
    for (ext, (fc, lc)) in &ext_counts {
        let lang = ext_to_language(ext);
        let e = lang_map.entry(lang).or_insert((0, 0));
        e.0 += fc;
        e.1 += lc;
    }

    let mut result: Vec<LanguageBreakdown> = lang_map.into_iter().map(|(language, (file_count, line_count))| {
        LanguageBreakdown {
            language,
            file_count,
            line_count,
            percent: line_count as f64 / total_lines as f64 * 100.0,
        }
    }).collect();
    result.sort_by(|a, b| b.line_count.cmp(&a.line_count));
    Ok(result)
}

fn ext_to_language(ext: &str) -> String {
    match ext {
        "rs" => "Rust", "ts" | "tsx" => "TypeScript", "js" | "jsx" => "JavaScript",
        "py" => "Python", "go" => "Go", "java" => "Java", "kt" => "Kotlin",
        "swift" => "Swift", "rb" => "Ruby", "c" => "C", "cpp" | "cc" => "C++",
        "h" => "C/C++ Header", "cs" => "C#", "sh" | "bash" | "zsh" | "fish" => "Shell",
        "toml" => "TOML", "yaml" | "yml" => "YAML", "json" => "JSON",
        "md" => "Markdown", "html" => "HTML", "css" | "scss" => "CSS", "sql" => "SQL",
        other => other,
    }.to_string()
}
```

### compute_top_files — git2 revwalk + diff pattern

```rust
pub fn compute_top_files(repo_path: &Path, limit: usize) -> anyhow::Result<Vec<FileChurnEntry>> {
    let repo = git2::Repository::open(repo_path)?;
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head().map_err(|_| anyhow::anyhow!("no HEAD"))?;
    revwalk.set_sorting(git2::Sort::TIME)?;

    let mut file_counts: std::collections::HashMap<String, usize> = Default::default();
    let mut walked = 0usize;

    for oid_result in revwalk {
        if walked >= 500 { log::debug!("compute_top_files: capped at 500 commits"); break; }
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;
        walked += 1;

        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            commit.parent(0).ok().and_then(|p| p.tree().ok())
        } else {
            None
        };

        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        diff.foreach(&mut |delta, _| {
            if delta.new_file().is_binary() { return true; }
            if let Some(p) = delta.new_file().path().and_then(|p| p.to_str()) {
                *file_counts.entry(p.to_string()).or_insert(0) += 1;
            }
            true
        }, None, None, None)?;
    }

    let mut entries: Vec<FileChurnEntry> = file_counts.into_iter()
        .map(|(path, change_count)| FileChurnEntry { path, change_count })
        .collect();
    entries.sort_by(|a, b| b.change_count.cmp(&a.change_count));
    entries.truncate(limit);
    Ok(entries)
}
```

Note: `revwalk.push_head()` returns Err on an empty repo (no commits). Catch this and return Ok(vec![]).

### find_db_repo_path SQL

```rust
pub fn find_db_repo_path(conn: &Connection, canonical: &Path) -> anyhow::Result<Option<String>> {
    let path_str = canonical.to_string_lossy();
    let result: rusqlite::Result<String> = conn.query_row(
        "SELECT repo_path FROM git_activity WHERE repo_path = ?1 OR repo_path LIKE ?1 || '/%' LIMIT 1",
        rusqlite::params![path_str],
        |row| row.get(0),
    );
    match result {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
```

### CLI command in cli.rs

```rust
/// Single-repo deep dive (language breakdown, top files, time invested, PR history)
Repo {
    /// Path to the git repository
    path: String,
    /// Output format: pretty, json
    #[arg(long, default_value = "pretty")]
    format: OutputFormat,
},
```

### main.rs match arm for Commands::Repo

```rust
Commands::Repo { path, format } => {
    let config = blackbox::config::load_config()?;
    let data_dir = blackbox::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = blackbox::db::open_db(&db_path)
        .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
    let dive = blackbox::repo_deep_dive::build_deep_dive(&path, &conn, &config)?;
    match format {
        OutputFormat::Pretty => blackbox::output::render_repo_pretty(&dive),
        OutputFormat::Json | OutputFormat::Csv => {
            println!("{}", blackbox::output::render_repo_json(&dive));
        }
    }
}
```

### render_repo_pretty — output sketch

```rust
pub fn render_repo_pretty(dive: &crate::repo_deep_dive::RepoDeepDive) {
    use colored::Colorize;
    println!("{}", format!("=== {} ===", dive.repo_name).bold().cyan());
    if !dive.tracked {
        println!("{}", "(untracked — no activity in DB)".dimmed());
    }
    println!();

    // Stats block
    let fmt_opt_date = |d: Option<chrono::DateTime<chrono::Utc>>| {
        d.map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "-".to_string())
    };
    println!("{:<20} {}", "Tracked commits:", dive.total_commits);
    println!("{:<20} {}", "Total time:", format_duration(dive.total_estimated_time));
    println!("{:<20} {}", "First commit:", fmt_opt_date(dive.first_commit_at));
    println!("{:<20} {}", "Last commit:", fmt_opt_date(dive.last_commit_at));
    println!();

    // Languages
    if !dive.languages.is_empty() {
        println!("{}", "Languages:".bold());
        let colors = ["cyan","green","yellow","magenta","blue","red","white"];
        for (i, lang) in dive.languages.iter().take(8).enumerate() {
            let filled = (lang.percent / 100.0 * 20.0).round() as usize;
            let bar = "█".repeat(filled) + &"░".repeat(20 - filled);
            let bar_colored = match colors[i % colors.len()] {
                "cyan"    => bar.cyan().to_string(),
                "green"   => bar.green().to_string(),
                "yellow"  => bar.yellow().to_string(),
                "magenta" => bar.magenta().to_string(),
                "blue"    => bar.blue().to_string(),
                "red"     => bar.red().to_string(),
                _         => bar.white().to_string(),
            };
            println!("  {:<16} {:>5.1}%  {}", lang.language, lang.percent, bar_colored);
        }
        println!();
    }

    // Top files
    if !dive.top_files.is_empty() {
        println!("{}", "Top files changed:".bold());
        for f in dive.top_files.iter().take(10) {
            println!("  {:>4}x  {}", f.change_count, f.path);
        }
        println!();
    }

    // Branches
    if !dive.branches.is_empty() {
        println!("{}", "Branches:".bold());
        for b in &dive.branches {
            println!("  {} — {} commits, last active {}",
                b.name,
                b.commit_count,
                b.last_active.format("%Y-%m-%d"));
        }
        println!();
    }

    // PRs
    if !dive.prs.is_empty() {
        println!("{}", "Pull requests:".bold());
        for pr in dive.prs.iter().take(10) {
            let state_str = match pr.state.as_str() {
                "MERGED" => format!("[{}]", pr.state).magenta().to_string(),
                "OPEN"   => format!("[{}]", pr.state).green().to_string(),
                "CLOSED" => format!("[{}]", pr.state).dimmed().to_string(),
                _        => format!("[{}]", pr.state).cyan().to_string(),
            };
            println!("  #{} {} {}", pr.number, pr.title, state_str);
        }
        println!();
    }
}
```

### render_repo_json

```rust
#[derive(serde::Serialize)]
pub struct JsonRepoDeepDive {
    pub repo_path: String,
    pub repo_name: String,
    pub tracked: bool,
    pub total_commits: usize,
    pub total_estimated_minutes: i64,
    pub first_commit_at: Option<String>,
    pub last_commit_at: Option<String>,
    pub languages: Vec<crate::repo_deep_dive::LanguageBreakdown>,
    pub top_files: Vec<crate::repo_deep_dive::FileChurnEntry>,
    pub branches: Vec<JsonBranchActivity>,
    pub prs: Vec<crate::repo_deep_dive::RepoPrEntry>,
}

#[derive(serde::Serialize)]
pub struct JsonBranchActivity {
    pub name: String,
    pub commit_count: usize,
    pub last_active: String,
}

pub fn render_repo_json(dive: &crate::repo_deep_dive::RepoDeepDive) -> String {
    let json = JsonRepoDeepDive {
        repo_path: dive.repo_path.clone(),
        repo_name: dive.repo_name.clone(),
        tracked: dive.tracked,
        total_commits: dive.total_commits,
        total_estimated_minutes: dive.total_estimated_time.num_minutes(),
        first_commit_at: dive.first_commit_at.map(|dt| dt.to_rfc3339()),
        last_commit_at: dive.last_commit_at.map(|dt| dt.to_rfc3339()),
        languages: dive.languages.clone(),
        top_files: dive.top_files.clone(),
        branches: dive.branches.iter().map(|b| JsonBranchActivity {
            name: b.name.clone(),
            commit_count: b.commit_count,
            last_active: b.last_active.to_rfc3339(),
        }).collect(),
        prs: dive.prs.clone(),
    };
    serde_json::to_string_pretty(&json).expect("JSON serialization should not fail")
}
```

Add `#[derive(Clone, serde::Serialize)]` to `LanguageBreakdown`, `FileChurnEntry`, `BranchActivity`, and `RepoPrEntry` in repo_deep_dive.rs.

### build_deep_dive orchestration

```rust
pub fn build_deep_dive(repo_path_input: &str, conn: &rusqlite::Connection, config: &crate::config::Config) -> anyhow::Result<RepoDeepDive> {
    let canonical = resolve_repo_path(repo_path_input)?;
    let db_repo_path = find_db_repo_path(conn, &canonical)?;
    let tracked = db_repo_path.is_some();
    let effective_path = db_repo_path.as_deref().unwrap_or(&canonical.to_string_lossy());
    // Note: need to own the string for the None case
    let effective_path_owned = db_repo_path.unwrap_or_else(|| canonical.to_string_lossy().to_string());

    let data = query_repo_all_time(conn, &effective_path_owned)?;

    let languages = compute_language_breakdown(&canonical).unwrap_or_default();
    let top_files = compute_top_files(&canonical, 10).unwrap_or_default();
    let total_estimated_time = compute_time_invested(&data, config.session_gap_minutes, config.first_commit_minutes);
    let branches = compute_branch_activity(&data);
    let prs = fetch_repo_pr_history(&effective_path_owned, &data);

    let commit_times: Vec<_> = data.events.iter()
        .filter(|e| e.event_type == "commit")
        .map(|e| e.timestamp)
        .collect();
    let first_commit_at = commit_times.iter().copied().min();
    let last_commit_at = commit_times.iter().copied().max();
    let total_commits = commit_times.len();

    let repo_name = std::path::Path::new(&effective_path_owned)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| effective_path_owned.clone());

    Ok(RepoDeepDive {
        repo_path: effective_path_owned,
        repo_name,
        total_commits,
        first_commit_at,
        last_commit_at,
        total_estimated_time,
        languages,
        top_files,
        branches,
        prs,
        tracked,
    })
}
```

---

## Edge Cases

- **Repo not in DB**: `find_db_repo_path` returns None. `tracked=false`. `data` has all empty vecs. All computed fields default to zero/empty. Output shows `(untracked — no activity in DB)`. Do NOT error — this is a valid state.
- **Repo path resolution/normalization**: User may pass `"."`, `"./subdir/.."`, `"~/code/myrepo"`, or an absolute path. `std::fs::canonicalize()` resolves symlinks and `..`. If the path doesn't exist or isn't readable, return Err before opening git2.
- **Worktree paths**: A user may pass a worktree path. `git2::Repository::open()` succeeds on worktrees. The DB may have the main repo path (not worktree). `find_db_repo_path` searches for both exact match and prefix — this covers the common case. If worktree path differs entirely from main repo path (no prefix relationship), activity will show as untracked. This is acceptable for now.
- **Repo with no commits tracked (untracked)**: Empty DB data. `compute_language_breakdown` and `compute_top_files` still run on the live git repo and return real data. Time invested = zero. Branches from DB = empty.
- **Binary files in language breakdown**: `blob.is_binary()` in git2 returns true for blobs with null bytes. Always skip these. Also skip files with no extension (no language mapping possible).
- **Empty repo (no commits at all)**: `repo.head()` returns Err. `compute_language_breakdown` returns Ok(vec![]). `compute_top_files`: `revwalk.push_head()` returns Err — catch and return Ok(vec![]).
- **Large repos**: `compute_top_files` is capped at 500 commits. `compute_language_breakdown` walks HEAD tree once — O(files). Both should complete in < 2s for normal repos.
- **gh CLI not available**: `fetch_repo_pr_history` degrades gracefully — returns review-only list from DB (may be empty). Never errors.
- **DB locked**: `open_db()` sets busy_timeout=5000ms. Caller in main.rs propagates the error if DB is unreachable.
- **Repo name from path**: Use `Path::file_name()` on the resolved canonical path, same as `query_activity` does for `repo_name`.
- **Rust 2024 / unsafe set_var in tests**: Wrap any `std::env::set_var` calls in tests in `unsafe {}`.
- **test git repos**: Use `git2::Repository::init()` not bare `std::fs::create_dir`. To create a commit in tests, use git2: `repo.index()`, `index.add_path()`, `index.write()`, `repo.find_object(index.write_tree()?, ...)`, `repo.commit(...)`.

---

## Files Changed

### New
- `src/repo_deep_dive.rs` — all deep-dive logic

### Modified
- `src/lib.rs` — add `pub mod repo_deep_dive;`
- `src/cli.rs` — add `Commands::Repo` variant
- `src/main.rs` — add match arm for `Commands::Repo`
- `src/output.rs` — add `render_repo_pretty()`, `render_repo_json()`, `JsonRepoDeepDive`, `JsonBranchActivity`

### New test files
- `tests/repo_deep_dive_test.rs` — US-012, US-013, US-014, US-015

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
- Do not add new crate dependencies — git2, rusqlite, chrono, colored, serde, serde_json are all already in Cargo.toml.
- Do not add `async` — entire codebase is synchronous blocking.
- `compute_language_breakdown` and `compute_top_files` must never panic on malformed repos — return Ok(vec![]) on any git2 error after the initial open.
- The `fetch_repo_pr_history` gh subprocess must use the same OnceLock + thread + recv_timeout pattern as enrichment.rs — no blocking forever.
- When adding `Commands::Repo`, search all `match` arms and `matches!` calls in cli.rs and main.rs for completeness — compiler will catch missing arms but be proactive.
