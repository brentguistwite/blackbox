# Agent Instructions: context-switch-frequency

## Branch
`feature/context-switch-frequency` — never commit to main.

## Task Loop (repeat for each user story)
1. Read the story from prd.json (id, acceptanceCriteria, files, deps)
2. Verify deps are complete before starting
3. Read all files listed in `files` before writing any code
4. Write tests first (TDD) — tests should fail before implementation
5. Implement until tests pass
6. Run quality checks (see below)
7. Fix any check failures
8. Commit with concise message referencing story id (e.g. `feat: US-CS-01 populate source_branch on branch_switch`)
9. Move to next story

## Quality Checks (run after every story)
```bash
cargo build 2>&1
cargo test 2>&1
cargo clippy -- -D warnings 2>&1
```
All three must pass clean before moving on.

## Project Context

### How branch switches are detected and stored
- **Detection**: `src/git_ops.rs` `poll_repo()` — compares `state.last_head_branch` (previous branch name, `Option<String>`) with `current_branch` (from `repo.head()?.shorthand()`).
- **Condition**: fires only when `state.last_head_branch.is_some()` (skips first poll to avoid false positive on daemon start).
- **DB write**: calls `db::insert_activity()` with `event_type="branch_switch"`, `branch=current_branch` (destination), `source_branch=None` (BUG: should be `state.last_head_branch.as_deref()`).
- **Schema**: `git_activity` table — `event_type TEXT CHECK(event_type IN ('commit','branch_switch','merge'))`, `branch TEXT` (destination), `source_branch TEXT` (migration M2, currently NULL for all branch_switch rows).
- **No dedup index**: branch_switch events use plain `INSERT` (not `INSERT OR IGNORE`) because `commit_hash` is NULL and the partial unique index only applies to non-NULL commit_hash.

### Key structs
- `RepoSummary` (query.rs:209) — add `branch_switches: usize`
- `ActivitySummary` (query.rs:222) — add `total_branch_switches: usize`
- `ActivityEvent` (query.rs:183) — already has `event_type`, `branch`, `timestamp`; branch_switch events are already included in `query_activity()` results

### query_activity() flow (query.rs:307)
Fetches all `git_activity` rows in `[from, to]` grouped by `repo_path` into `repo_map: BTreeMap<String, Vec<ActivityEvent>>`. Each `ActivityEvent` includes `event_type`. branch_switch events are already present in `events` — no additional DB query needed, just filter `event_type == "branch_switch"` and run through the noise filter.

### Output pipeline
- `run_query()` in main.rs builds `ActivitySummary`, calls `render_summary()` / `render_json()` / `render_csv()`
- `render_summary_to_string()` in output.rs builds the pretty summary string — inject switch line here
- `render_standup()` in output.rs — inject at Total line if threshold met

### Existing test patterns
- `tempfile::TempDir` for DB paths
- `git2::Repository::init()` for test repos
- `assert_cmd::Command::cargo_bin("blackbox")` for CLI integration tests
- Rust 2024: `unsafe { std::env::set_var(...) }` required in tests that set env vars

## Edge Cases to Handle

### US-CS-01: source_branch population
- `state.last_head_branch` is `None` on first poll — the guard `if state.last_head_branch.is_some()` already prevents recording, so no NULL issue
- Worktree repos: `db_repo_path` is the main repo path, branch detection works the same way

### US-CS-03: noise filtering
- **Detached HEAD**: `current_branch` is `None` when `repo.head_detached()` is true. These events have `branch=None` in DB. Exclude any switch where `to_branch` is None.
- **Rebase sequences**: typically emit `main -> detached -> feature` — detached HEAD exclusion handles most of this.
- **Same-branch re-checkout**: `from_branch == to_branch` (possible if branch pointer moved but name is same). Exclude.
- **Round-trip pairs** A→B→A: if total elapsed < `min_dwell_secs` (default 120s), collapse to 0 net switches. Algorithm: sliding window — if switch[i].to == switch[i-2].to and switch[i].timestamp - switch[i-1].timestamp < threshold, remove both.
- **Rapid multi-hop**: A→B→C→A within window — treat conservatively: only collapse exact round-trips, keep others.
- **NULL source_branch in existing rows**: filter algorithm must handle `Option<String>` for both from/to fields.

### US-CS-07: `blackbox focus` command
- `Commands::Focus` must be added to `Commands::is_exempt_from_config_check` list? No — it requires DB access, so config must exist. Leave as non-exempt.
- Zero-switch case must print friendly message, not crash.

## Documentation Updates
The final story (US-CS-09) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## Implementation Order
US-CS-01 → US-CS-02 → US-CS-03 → US-CS-04 → US-CS-05 → US-CS-06 → US-CS-07 → US-CS-08

P0 stories are blocking. P1 is core value. P2/P3 are enhancements — skip if scope is tight.

## Focus Cost Constant
23 minutes per switch (Gloria Mark, UC Irvine research). Define as `const FOCUS_COST_PER_SWITCH_MINS: i64 = 23;` in query.rs or output.rs — do not hardcode inline.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.
