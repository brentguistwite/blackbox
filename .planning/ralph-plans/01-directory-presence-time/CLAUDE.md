# Ralph Agent Instructions — directory-presence-time

You are an autonomous coding agent implementing the `directory-presence-time` feature for the blackbox CLI. Work story-by-story, write tests first (TDD), verify each story before moving to the next.

---

## Your Task

Follow these steps for each story:

1. **Read** the story from `prd.json` and identify acceptance criteria.
2. **Write tests first** — add failing tests to `tests/query_test.rs` (or `tests/time_test.rs` for pure unit tests) before touching implementation.
3. **Implement** the minimal change to make tests pass.
4. **Run quality checks** (see below) — fix all failures before proceeding.
5. **Verify** acceptance criteria are met.
6. **Report** status (see Progress Report Format).
7. Repeat for the next story.

Never implement multiple stories simultaneously. Never skip to implementation before tests exist.

---

## Story Dependencies

```
US-001  (no deps)
  └─ US-002
       └─ US-003
            └─ US-004
US-002 ─── US-005  (docs, can be done last)
```

Implement in order: US-001 → US-002 → US-003 → US-004 → US-005.

---

## Quality Checks

Run these after every story. All must pass before marking a story done.

```bash
cargo build 2>&1
cargo test  2>&1
cargo clippy -- -D warnings 2>&1
```

Fix all errors and warnings before proceeding.

---

## Project Context

**Crate:** `blackbox-cli` (lib name `blackbox`, binary `blackbox`). Edition 2024. SQLite-backed, single crate.

**Key files:**
- `src/query.rs` — all time estimation logic lives here
- `src/db.rs` — `record_directory_presence`, `open_db` with migrations
- `tests/query_test.rs` — integration tests for query_activity, query_presence, estimate_time_v2
- `tests/time_test.rs` — pure unit tests for estimate_time (legacy)

**Relevant functions (all in `src/query.rs`):**

```rust
// Public — change signature in US-001
pub fn estimate_time_v2(
    events: &[ActivityEvent],
    ai_sessions: &[TimeInterval],
    session_gap_minutes: u64,
    first_commit_minutes: u64,
) -> (Duration, Vec<TimeInterval>)

// Public — call query_presence inside this in US-003
pub fn query_activity(conn, from, to, session_gap_minutes, first_commit_minutes)
    -> anyhow::Result<Vec<RepoSummary>>

// Public — already written, zero callers outside tests
pub fn query_presence(conn, from, to, session_gap_minutes)
    -> anyhow::Result<BTreeMap<String, Vec<TimeInterval>>>

// Public — update in US-004
pub fn global_estimated_time(repos: &[RepoSummary], ...) -> Duration
```

**`TimeInterval` struct** (copy/paste safe):
```rust
pub struct TimeInterval {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}
```

**`RepoSummary` struct** — add `presence_intervals: Vec<TimeInterval>` in US-004. When adding a field, search all test files for `RepoSummary {` constructions and update them — the compiler will flag every site.

**`estimate_time_v2` current logic (relevant to US-002):**
- Step 1: compute adaptive `effective_gap` / `effective_credit` from median commit gap
- Step 2: group git events into sessions → tentative intervals `[first_event - effective_credit, last_event]`
- Step 3: credit suppression — if AI session overlaps the credit window `[tentative_start, first_event]`, shrink git interval start to `first_event`
- Step 4: collect git + AI intervals
- Step 5: merge and return

**US-002 anchoring logic to add (new step between 2 and 3):**
- For each git session interval, check if any presence interval starts before `first_event - effective_credit`
  AND overlaps the window `[first_event - effective_credit, first_event]`
- If yes: set `interval.start = presence.start` (presence anchors the session start)
- Insert this as Step 2b, before the existing AI credit-suppression (Step 3)

**`query_presence` behavior:**
- Returns `BTreeMap<String, Vec<TimeInterval>>` keyed by `repo_path`
- `NULL left_at` is capped at `entered_at + session_gap_minutes`
- All intervals already clipped to `[from, to]`
- Does not create standalone repo entries — presence-only repos must NOT appear in `query_activity` results (existing test enforces this)

---

## Edge Cases

- Repo with presence data but zero commits: presence intervals pass to `estimate_time_v2` but since no git events exist, presence simply contributes its own duration (US-001 test covers this).
- Presence interval starts before query window: `query_presence` clips to `from`, so `estimate_time_v2` sees a clipped interval — no special handling needed.
- Multiple presence intervals for a repo: pass all of them; `merge_intervals` at the end deduplicates overlaps.
- Presence covers entire work day with only one commit: presence anchors the session start well before the single commit; effective_credit still applies if no presence overlap.
- `global_estimated_time` must not double-count time worked simultaneously in two repos: `merge_intervals` across all repos handles this; presence intervals from different repos may overlap but are merged globally.

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
The final story (US-006) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Branch is `feature/directory-presence-time`.
- Never skip a story or reorder without noting a blocker.
- Never use `estimate_time` (legacy function) — only `estimate_time_v2`.
- Presence data must NOT create new repo entries in `query_activity` output (existing test: `query_activity_presence_does_not_create_repos`).
- When changing `estimate_time_v2` signature, grep for all call sites and update all of them before running tests.
- Edition 2024: `unsafe` block required for `std::env::set_var` if used in tests.
- Use `tempfile::NamedTempFile` + `open_db` for all DB tests.
- Follow existing test helper patterns: `ts(h, m)` and `iv(h1, m1, h2, m2)` are already defined in `tests/query_test.rs` — reuse them.
