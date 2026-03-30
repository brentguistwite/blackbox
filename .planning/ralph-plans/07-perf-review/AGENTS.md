# Agent Instructions: blackbox perf-review

## Branch
Always work on `feature/perf-review`. Never commit to `main`.

## Task Loop (9 steps — repeat until all stories Done)

1. **Read** the PRD (`prd.json`) and identify the next unstarted story in priority order (P0 first, then P1).
2. **Explore** relevant source files before writing any code.
3. **Write tests first** where the story has testable unit logic (US-04 theme extraction, US-05 PR dedup, US-07 context builder). Skip test-first only for pure wiring stories (US-08, US-11 module declaration).
4. **Implement** the story. Keep diffs minimal — reuse existing patterns.
5. **Build**: `cargo build` — fix all errors before proceeding.
6. **Lint**: `cargo clippy -- -D warnings` — fix all warnings.
7. **Test**: `cargo test` — all tests must pass.
8. **Mark story done** by adding a comment at top of the relevant source file section (or in your scratchpad). Do NOT edit prd.json.
9. **Loop** to step 1 for next story.

## Quality Gates (must pass before any commit)
```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

## Project Context

**Crate**: `blackbox-cli` (lib name: `blackbox`, binary: `blackbox`)
**Rust edition**: 2024 — use `let-else`, `if-let` chains, `unsafe` block for `std::env::set_var` in tests.
**Entry point**: `src/main.rs` → Clap derive dispatch.

### Existing patterns to follow

**Adding a CLI command** (US-08):
1. Add variant to `Commands` enum in `src/cli.rs`
2. Add match arm in `src/main.rs`
3. Do NOT add to `is_exempt_from_config_check`

**LLM integration** (US-06):
- `src/llm.rs` has `build_llm_config(&Config) -> anyhow::Result<LlmConfig>`
- `stream_anthropic` / `stream_openai` / `parse_sse_stream` are private — add a new pub fn `generate_perf_review` in `src/llm.rs` (or `src/perf_review.rs`) that calls them, or duplicate the call pattern for the new system prompt.
- `max_tokens`: use 2048 for perf-review (existing summaries use 1024).
- Anthropic header: `"anthropic-version": "2023-06-01"`, `"x-api-key"`.
- OpenAI: Bearer token, base_url from config.

**Date ranges** (US-01/US-02):
- Follow `today_range` / `week_range` / `month_range` pattern in `src/query.rs`.
- All ranges return `(DateTime<Utc>, DateTime<Utc>)`.
- Convert local midnight to UTC via `Local.from_local_datetime(...).unwrap().with_timezone(&Utc)`.

**Query activity** (US-03):
- `query_activity(conn, from, to, config.session_gap_minutes, config.first_commit_minutes)` returns `Vec<RepoSummary>`.
- Follow `run_query` helper in `main.rs` as the pattern for orchestration.
- `enrich_with_prs(&mut repos)` — call after query; silently no-ops if gh unavailable.

**New module** (US-11):
- Create `src/perf_review.rs`.
- Add `pub mod perf_review;` to `src/lib.rs`.
- Reference in `main.rs` as `blackbox::perf_review::`.

**Integration tests** (US-12):
- Files in `tests/` are integration tests with access to the `blackbox` lib crate.
- Use `assert_cmd::Command::cargo_bin("blackbox")`.
- Use `tempfile::TempDir` for temp DB; set env var `BLACKBOX_DATA_DIR` if the binary respects it, otherwise insert records directly via `rusqlite` in test setup.
- Check: does binary read `BLACKBOX_DATA_DIR`? If not, tests may need to use `--db-path` flag or a test config. Inspect `src/config.rs::data_dir()` — it uses `etcetera` XDG, no env override exists yet. To avoid needing to add one: test the library functions directly (unit test `build_perf_review_context`, etc.) rather than the binary for data-dependent tests. For CLI flag validation tests (bad date, missing API key), no DB needed.

### Key structs (read before implementing)

**`ActivitySummary`** (`src/query.rs:222`):
```
period_label, total_commits, total_reviews, total_repos,
total_estimated_time, total_ai_session_time, repos: Vec<RepoSummary>
```

**`RepoSummary`** (`src/query.rs:209`):
```
repo_path, repo_name, commits, branches: Vec<String>,
estimated_time: Duration, events: Vec<ActivityEvent>,
pr_info: Option<Vec<PrInfo>>, reviews: Vec<ReviewInfo>,
ai_sessions: Vec<AiSessionInfo>
```

**`ActivityEvent`** (`src/query.rs:183`): `event_type, branch, commit_hash, message, timestamp`

**`ReviewInfo`** (`src/query.rs:192`): `pr_number, pr_title, action, reviewed_at`

**`PrInfo`** (`src/enrichment.rs:9`): `number, title, state, head_ref_name`

**`Config`** (`src/config.rs:23`): `llm_api_key, llm_provider, llm_model, llm_base_url, session_gap_minutes, first_commit_minutes`

## Documentation Updates
The final story (US-13) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## Edge Cases — Handle All of These

| Scenario | Required behavior |
|---|---|
| No LLM API key | `build_llm_config` returns `Err`; print helpful TOML snippet; exit 1 |
| Zero commits + zero reviews | Return error before LLM call: "No activity found for this period." |
| < 10 commits | Prepend warning to LLM prompt about limited data window |
| gh CLI unavailable | `pr_info = None`, `reviews = []`; no error; LLM prompt notes "PR data unavailable" |
| Quarter with massive commit volume (>200 commits) | Truncate to 200 most-recent commit messages in context; note in prompt |
| Serialized context > 40,000 chars | Truncate commit list, append note to prompt |
| Invalid `--from`/`--to` date | `anyhow::bail!` with "Invalid date: expected YYYY-MM-DD" |
| `--from` after `--to` | `anyhow::bail!` with "from must be before to" |
| No commits but has reviews | Proceed — reviews alone are valid input |
| Rust 2024 `unsafe` in tests | Wrap `std::env::set_var` in `unsafe {}` block |

## LLM Prompt Design (US-06)

System prompt key requirements:
- First-person, professional, performance-review tone
- ~400-600 words output
- Markdown sections: `## Summary`, `## Key Contributions`, `## Technical Themes`, `## Collaboration & Code Review`, `## Time Investment`
- Do NOT invent accomplishments not evidenced in the data
- If PR data is absent, omit or note "PR data unavailable"

User message structure (serialize `PerfReviewContext` to JSON, prefix with instruction):
```
Generate a performance review self-assessment for this developer's activity:

{context_json}
```

## Theme Extraction (US-04)

Stop-word list (at minimum): fix, the, a, an, add, added, update, updated, chore, feat, feature, refactor, test, tests, wip, merge, merged, bump, release, minor, major, revert, pr, co, authored, by, for, in, of, on, to, use, used, using, remove, removed, change, changes, clean, cleanup, typo, misc

Implementation: lowercase all messages, split on non-alphanumeric, filter stop-words and words < 3 chars, count frequency, return top 20 sorted desc by count.

## Files to Create/Modify

| File | Action |
|---|---|
| `src/perf_review.rs` | CREATE — main module |
| `src/lib.rs` | MODIFY — add `pub mod perf_review;` |
| `src/cli.rs` | MODIFY — add `PerfReview` variant |
| `src/main.rs` | MODIFY — add match arm |
| `src/query.rs` | MODIFY — add `quarter_range()` |
| `src/llm.rs` | MODIFY (optional) — may add `PERF_REVIEW_SYSTEM_PROMPT` here or in `perf_review.rs` |
| `tests/perf_review.rs` | CREATE — integration tests |

## Implementation Order

1. US-11 (module skeleton) — unblocks everything
2. US-01 (quarter_range) — needed by US-02
3. US-07 (PerfReviewContext struct + builder) — write unit test first
4. US-04 (theme extraction) — write unit test first
5. US-05 (PR summary compilation) — write unit test first
6. US-02 (CLI flags + date parsing)
7. US-03 (aggregation wiring)
8. US-06 (LLM prompt + generate fn)
9. US-08 (CLI command + main.rs wiring)
10. US-09 (sparse data handling) — add to US-06 implementation
11. US-10 (API key error UX) — verify during US-08
12. US-12 (integration tests)

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.
