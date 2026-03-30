# Ralph Agent Instructions — llm-insights

You are an autonomous coding agent implementing the `llm-insights` feature for the `blackbox` CLI. Read prd.json in this directory for the full story list. Implement stories in dependency order. Do not commit to `main` — always work on `feature/llm-insights`.

---

## Your Task

Follow these steps in order:

1. **Read prd.json** — understand all stories, priorities, deps before writing any code.
2. **Create feature branch**: `git checkout -b feature/llm-insights`
3. **Implement P0 stories first** (US-001, US-002, US-003, US-004, US-005, US-006, US-009, US-013, US-014, US-016, US-017), then P1 (US-007, US-008, US-010, US-011, US-015, US-018), then P2 (US-012).
4. **Within each story**: write the test first (TDD), then implement until the test passes.
5. **After each story**: `cargo build` must pass before moving to the next story.
6. **After all stories**: run `cargo test` and `cargo clippy` and fix all failures/warnings.
7. **Do not fix tests after every iteration** during rapid implementation — batch test fixes at the end of each priority tier, or when explicitly asked.
8. **Write a progress report** after completing each story (see format below).
9. **Do not push** unless the user asks.

---

## Story Dependencies

```
US-001 (data aggregation)
  ├─ US-002 (prompt construction)
  │    ├─ US-008 (token budget)
  │    │    └─ US-014 (test: prompt)
  │    └─ US-012 (msg length trend)
  ├─ US-006 (JSON serialization)
  │    └─ US-016 (CLI integration test: --format json)
  └─ US-011 (PR merge time enrichment)
US-013 depends on US-001

US-003 (insights system prompt)
  └─ US-004 (LLM API call fn)
       └─ US-005 (CLI: blackbox insights)
            ├─ US-007 (config: insights settings)
            ├─ US-009 (empty activity short-circuit)
            │    └─ US-017 (test: empty short-circuit)
            ├─ US-010 (progress indicator)
            └─ US-018 (test: missing API key)
US-015 depends on US-003
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

- `src/cli.rs` — Commands enum, clap derive pattern. Add `Insights` variant here.
- `src/main.rs` — match arm dispatch. Add arm for `Commands::Insights { window, format }`.
- `src/query.rs` — all DB query fns live here. Add `aggregate_insights_data` here.
- `src/output.rs` — all render fns live here. Add `render_insights_json` here.
- `src/llm.rs` — existing LLM integration. Add `INSIGHTS_SYSTEM_PROMPT`, `generate_insights`, `build_insights_prompt`, `truncate_repos_for_prompt` here.
- `src/config.rs` — Config struct. Add `insights_max_tokens`, `insights_window` fields here.
- `src/enrichment.rs` — `enrich_with_prs`, `PrInfo`. Call in aggregate_insights_data for PR merge times.
- `src/lib.rs` — module declarations. No new module needed — insights logic lives in existing modules.
- `tests/query_test.rs` — integration test pattern: `setup_db()` helper, `insert_activity(...)`. Copy or factor out for use in insights_test.rs.

### DB schema (relevant tables)

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

Timestamps are RFC3339 UTC strings. Parse with `DateTime::parse_from_rfc3339` + `.with_timezone(&Utc)`, then `.with_timezone(&Local)` for local-time bucketing.

### Existing LLM patterns to follow

**LlmConfig** (src/llm.rs) — already built by `build_llm_config(config: &Config)`. Reuse for insights.

**Provider routing** — the existing `summarize_activity` fn dispatches on `config.provider` to `stream_anthropic` / `stream_openai`. Do the same in `generate_insights`. The private `stream_anthropic` and `stream_openai` fns are not pub — you will need to either make them pub(crate) or duplicate minimal wrappers. Prefer refactoring: extract a `call_llm_streaming(client, config, system_prompt, user_content, max_tokens) -> anyhow::Result<()>` helper that both `summarize_activity` and `generate_insights` call.

**SSE streaming** — `parse_sse_stream` in llm.rs is already generic over an extractor closure. Reuse directly.

**Rate limit detection**: in the response handler, check `resp.status() == 429` before the generic error bail.

**Timeout**: use `Duration::from_secs(60)` for insights (vs 30s for summarize).

### Existing output patterns to follow

`render_json` in output.rs builds intermediate JSON structs (JsonSummary, JsonRepo) rather than serializing domain types directly. For insights, make InsightsData + RepoInsights themselves `#[derive(serde::Serialize)]` — simpler because there's no chrono Duration serialization issue (all stats are primitives).

### CLI variant pattern (cli.rs)

```rust
/// LLM-powered behavioral analysis of activity patterns
Insights {
    /// Time window to analyze: week or month (default: week)
    #[arg(long, default_value = "week", value_parser = ["week", "month"])]
    window: String,
    /// Output format: pretty (LLM stream) or json (raw data, no LLM call)
    #[arg(long, default_value = "pretty")]
    format: OutputFormat,
},
```

### main.rs dispatch pattern

```rust
Commands::Insights { window, format } => {
    blackbox::insights::run_insights(&window, format)?;
}
```

Add `pub mod insights;` to `src/lib.rs` and create `src/insights.rs` as the orchestration module:

```rust
// src/insights.rs
pub fn run_insights(window: &str, format: OutputFormat) -> anyhow::Result<()> {
    let config = crate::config::load_config()?;
    let data_dir = crate::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = crate::db::open_db(&db_path)?;

    let (from, to) = match window {
        "month" => crate::query::month_range(),
        _ => crate::query::week_range(),
    };
    let window_label = match window { "month" => "This Month", _ => "This Week" };

    let mut repos = crate::query::query_activity(&conn, from, to, config.session_gap_minutes, config.first_commit_minutes)?;
    crate::enrichment::enrich_with_prs(&mut repos);

    let data = crate::query::aggregate_insights_data(&repos, window_label);

    match format {
        OutputFormat::Json => {
            println!("{}", crate::output::render_insights_json(&data));
        }
        OutputFormat::Pretty => {
            if data.total_commits == 0 && data.total_repos == 0 {
                println!("No activity recorded for {}. Nothing to analyze.", window_label);
                return Ok(());
            }
            eprintln!("Analyzing {} commits across {} repos...", data.total_commits, data.total_repos);
            let llm_config = crate::llm::build_llm_config(&config)?;
            let prompt = crate::llm::build_insights_prompt(&data);
            crate::llm::generate_insights(&llm_config, &prompt)?;
        }
        OutputFormat::Csv => anyhow::bail!("--format csv not supported for insights"),
    }
    Ok(())
}
```

Note: `aggregate_insights_data` takes `&[RepoSummary]` (already queried + enriched), not `&Connection`. This keeps it testable without a DB.

### InsightsData struct (query.rs)

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct RepoInsights {
    pub repo_name: String,
    pub commits: usize,
    pub estimated_minutes: i64,
    pub branches_touched: usize,
    pub has_prs: bool,
    pub avg_commit_msg_len: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InsightsData {
    pub period_label: String,
    pub total_commits: usize,
    pub total_repos: usize,
    pub commits_by_dow: [u32; 7],      // Mon=0..Sun=6, local time
    pub commits_by_hour: [u32; 24],    // 0..23, local time
    pub avg_msg_len_by_dow: [f64; 7],  // avg commit message char length per day
    pub bugfix_commits: u32,
    pub total_commits_with_msg: u32,
    pub pr_merge_times_hours: Vec<f64>, // only merged PRs with both timestamps
    pub per_repo: Vec<RepoInsights>,
}
```

Implement `aggregate_insights_data(repos: &[RepoSummary], period_label: &str) -> InsightsData`.

### Bug-fix classification

```rust
fn is_bugfix_commit(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    ["fix", "bug", "hotfix", "patch", "revert"]
        .iter()
        .any(|kw| lower.contains(kw))
}
```

### INSIGHTS_SYSTEM_PROMPT guidance

Write a system prompt that:
- Directs the LLM to generate 4-6 behavioral insights
- Requires each insight to start with a specific data reference ("On Tuesdays, X% of your commits...", "Your longest sessions averaged...")
- Prohibits generic advice ("Consider...", "It's recommended...", "Great job...")
- States the format: bullet list, 1-2 sentences per bullet, no headers
- Keeps it under 200 words

Example insight style the prompt should produce:
- "Tuesdays account for 28% of your commits this week — your highest-output day by a significant margin."
- "3x more bug-fix commits appear on Fridays vs Mondays (6 vs 2), suggesting late-week pressure."
- "Your average commit message is 12 chars shorter on Fridays than Mondays — possible end-of-week fatigue signal."

### build_insights_prompt format

Structure the user prompt as a compact text block, NOT JSON. Example:

```
Analyze these developer activity patterns and provide specific, data-driven behavioral insights:

Period: This Week
Total commits: 47 across 5 repos

Commits by day: Mon: 18% (8), Tue: 26% (12), Wed: 17% (8), Thu: 22% (10), Fri: 17% (8), Sat: 0% (0), Sun: 0% (0)
Peak commit hour: 10:00 (9 commits); top hours: 10, 14, 15
Bug-fix commits: 6/47 (13%)
Avg commit msg length by day: Mon: 42ch, Tue: 38ch, Wed: 35ch, Thu: 31ch, Fri: 28ch (trend: shorter toward end of week)

Top repos:
- api-gateway: 18 commits, ~3h 20m, 4 branches
- frontend: 12 commits, ~2h 10m, 2 branches [has PRs]
- infra: 9 commits, ~1h 45m, 1 branch
(showing top 3 of 5 repos)
```

Key rules:
- Use percentages + raw counts for DOW distribution
- "top hours" = top 3 by commit count
- Include `[has PRs]` marker for repos with PRs
- Include PR merge time section only if pr_merge_times_hours non-empty: "PR merge times: median Xh, range Y–Zh (N PRs)"
- Repo list capped at 10; add note if truncated

---

## Edge Cases

- **No API key configured**: `build_llm_config` returns descriptive error with config.toml example — let it propagate naturally via `?`. Do not add a separate check.
- **Rate limit (HTTP 429)**: bail with `'Rate limited by <provider>. Try again in a moment.'` — check status code before generic error handling.
- **Empty activity window**: short-circuit before LLM call (US-009). Check `data.total_commits == 0 && data.total_repos == 0`.
- **Token limits for large datasets**: truncate per_repo to top 10 by commit count; use compact prompt format (no raw commit lists). Full month of heavy activity should still fit in ~2000 prompt tokens.
- **No commit messages**: some events may have `message: None`. Skip for avg_msg_len_by_dow calculation and for bug-fix detection.
- **PR enrichment unavailable**: `gh` CLI not installed or no auth. `enrich_with_prs` degrades gracefully — pr_merge_times_hours will be empty. Prompt section for PR times simply omitted.
- **Single repo**: per_repo has 1 entry; no truncation note needed.
- **All commits on one day**: avg_msg_len_by_dow has 0.0 for other days — omit zero-days from the prompt's per-day listing (only include days with commits > 0).
- **Rust 2024**: if any test uses `std::env::set_var`, wrap in `unsafe {}`.
- **OutputFormat::Csv**: bail with unsupported message (insights has no CSV output).

---

## New Files

### src/insights.rs

Orchestration module. Contains `pub fn run_insights(window: &str, format: OutputFormat) -> anyhow::Result<()>`. Register in `src/lib.rs` with `pub mod insights;`.

### tests/insights_test.rs

Integration tests for US-013 through US-018. Use `setup_db()` pattern from query_test.rs. Since aggregate_insights_data takes `&[RepoSummary]` (not a Connection), unit tests for aggregation can construct RepoSummary values directly without DB setup — much faster.

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

After completing all stories in a priority tier:

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
- All new public types/fns must be `pub`.
- All structs used in JSON output must `#[derive(serde::Serialize)]`.
- No new DB migrations — this feature reads existing data only.
- No evaluative language in output ("Great job", "You should", "Consider"). The LLM prompt instructs this; the Rust code should not add any either.
- Prefer refactoring existing llm.rs helpers (extract `call_llm_streaming`) over duplicating Anthropic/OpenAI call logic.
- Keep output.rs growing in the existing file unless it exceeds ~800 lines.
