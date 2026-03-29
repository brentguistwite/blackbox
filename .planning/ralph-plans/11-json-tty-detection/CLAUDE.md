# Ralph Agent Instructions — json-tty-detection

You are an autonomous coding agent implementing the `json-tty-detection` feature for the blackbox CLI. Work story-by-story, write tests first (TDD), verify each story before moving to the next.

---

## Your Task

Follow these steps for each story:

1. **Read** the story from `prd.json` and identify acceptance criteria.
2. **Write tests first** — add failing tests to the appropriate test file before touching implementation.
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
  ├─ US-002
  │    ├─ US-003
  │    ├─ US-005
  │    └─ US-006  (docs, can be done last)
  └─ US-004
```

Implement in order: US-001 → US-004 → US-002 → US-003 → US-005 → US-006.

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
- `src/cli.rs` — `Commands` enum with Clap derive; Today/Week/Month/Standup have `format: OutputFormat` field
- `src/main.rs` — `run_query(period_label, range_fn, format, summarize)` dispatches all three query commands
- `src/output.rs` — `OutputFormat` enum, `render_summary`, `render_json`, `render_csv`, `render_standup`
- `tests/output_test.rs` (or create it) — output rendering tests
- `tests/cli_test.rs` — assert_cmd integration tests

**Current `OutputFormat` enum** (`src/output.rs` line 8):
```rust
#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Pretty,
    Json,
    Csv,
}
```

**Current `Commands` variants with format** (Today/Week/Month each have):
```rust
#[arg(long, default_value = "pretty")]
format: OutputFormat,
#[arg(long)]
summarize: bool,
```

**`run_query` signature in `main.rs`:**
```rust
fn run_query(
    period_label: &str,
    range_fn: fn() -> (DateTime<Utc>, DateTime<Utc>),
    format: OutputFormat,
    summarize: bool,
) -> anyhow::Result<()>
```

**Match arm pattern for Today (same for Week/Month):**
```rust
Commands::Today { format, summarize } => {
    run_query("Today", blackbox::query::today_range, format, summarize)?;
}
```

---

## Implementation Guide

### US-001: `is_tty()` in output.rs

Add to `src/output.rs`:
```rust
use std::io::IsTerminal;

/// Returns true when stdout is an interactive terminal.
/// Returns false when stdout is piped, redirected to a file, or non-interactive.
pub fn is_tty() -> bool {
    std::io::stdout().is_terminal()
}
```

`IsTerminal` is stable since Rust 1.70. No new dependencies.

### US-002: `--json`/`--csv` flags + `resolve_format`

Add to each of Today, Week, Month, Standup in `cli.rs`:
```rust
/// Emit JSON output (shorthand for --format json)
#[arg(long, conflicts_with = "csv")]
json: bool,
/// Emit CSV output (shorthand for --format csv)
#[arg(long, conflicts_with = "json")]
csv: bool,
```

Add `resolve_format` to `src/output.rs` (or `src/main.rs`):
```rust
pub fn resolve_format(format: OutputFormat, json: bool, csv: bool) -> OutputFormat {
    if json { return OutputFormat::Json; }
    if csv  { return OutputFormat::Csv; }
    format
}
```

Update match arms in `main.rs`:
```rust
Commands::Today { format, json, csv, summarize } => {
    let fmt = resolve_format(format, json, csv);
    run_query("Today", blackbox::query::today_range, fmt, summarize)?;
}
```

### US-003: TTY auto-detection in `resolve_format`

Extend `resolve_format` to accept `tty: bool` (pass `is_tty()` from call site):
```rust
pub fn resolve_format(format: OutputFormat, json: bool, csv: bool, tty: bool) -> OutputFormat {
    if json { return OutputFormat::Json; }
    if csv  { return OutputFormat::Csv; }
    // Auto-detect: non-TTY + no explicit format → JSON
    if !tty && matches!(format, OutputFormat::Pretty) {
        return OutputFormat::Json;
    }
    format
}
```

Call site: `resolve_format(format, json, csv, blackbox::output::is_tty())`.

Unit test for `resolve_format` (pure function, no TTY needed):
```rust
// tty=true, no flags → Pretty
// tty=false, no flags → Json
// tty=false, --format pretty explicit → Pretty  (cannot distinguish from default; see edge cases)
// tty=false, --json → Json
// tty=false, --csv → Csv
// tty=true, --csv → Csv
```

**Note on "explicit --format pretty" in non-TTY:** Clap `default_value = "pretty"` means there is no way to distinguish `--format pretty` (explicit) from the default. This is a known limitation. Document it in the flag help text. The user workaround is to not set `--format` and rely on TTY detection, or to pipe and add `--format pretty` knowing it will stay pretty.

### US-004: Strip ANSI in non-TTY

In `main.rs`, before command dispatch:
```rust
if !blackbox::output::is_tty() {
    colored::control::set_override(false);
}
```

`colored::control::set_override(false)` disables all ANSI output globally for the process. No new dependency — `colored` crate is already in `Cargo.toml`.

### US-005: Standup --json/--csv

`Commands::Standup` currently has `week: bool, summarize: bool`. Add:
```rust
#[arg(long, conflicts_with = "csv")]
json: bool,
#[arg(long, conflicts_with = "json")]
csv: bool,
```

Match arm update — reuse existing standup query block but add format resolution before the if-summarize block.

### US-006: Doc comments on --json

```rust
/// Output JSON. Shape: {period_label, total_commits, total_reviews, total_repos,
/// total_estimated_minutes, total_ai_session_minutes,
/// repos: [{repo_name, repo_path, commits, branches, estimated_minutes,
///          events, pr_info?, reviews, ai_sessions}]}
#[arg(long, conflicts_with = "csv")]
json: bool,
```

---

## Documentation Updates
The final story (US-007) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## Edge Cases

- **`--json` + `--format json`:** Both set — clap does NOT conflict these. `resolve_format` returns Json either way. No user-visible issue.
- **`--json` + `--csv` together:** `conflicts_with` in clap causes a hard error with a clear message. This is intentional.
- **`--format pretty` in a pipe:** Cannot distinguish from default. User gets Json. Document in `--format` help text: "In non-TTY mode, defaults to json unless explicitly set."
- **`--summarize` flag:** Takes precedence over format selection entirely (current behavior in `run_query`). TTY detection does not affect summarize path.
- **`render_standup` with --json flag:** `render_standup` is a plain-text format. When `--json` is passed on standup, skip `render_standup` and call `render_json` instead. Both functions accept `&ActivitySummary`.
- **CSV empty output:** `render_csv` already handles empty repos (writes header only). No special case needed.
- **`Live` / `Doctor` / `Status` commands:** These do not have format flags and are not in scope for this feature. TTY detection for ANSI stripping (US-004) still applies to them automatically via `colored::control::set_override`.
- **colored crate and NO_COLOR env:** `colored` already respects `NO_COLOR=1` and `TERM=dumb`. US-004 is additive — it also disables color when piped, regardless of env vars.
- **Test environment:** `assert_cmd` tests spawn a subprocess. The subprocess stdout is a pipe (not a TTY). This means US-003 auto-detection is exercised naturally in all assert_cmd tests that don't pass `--format pretty` explicitly. Be aware: existing tests that assert pretty output will break if they don't pass `--format pretty`. Audit existing CLI tests when implementing US-003.

---

## Test File Guidance

- Pure unit tests for `resolve_format` and `is_tty` compilation: `tests/output_test.rs` (create if absent)
- CLI integration tests: `tests/cli_test.rs` (existing pattern with `assert_cmd::Command::cargo_bin("blackbox")`)
- Rust 2024: `unsafe` block required for `std::env::set_var` in tests
- Use `assert_fs::TempDir` + env var overrides for config in CLI tests (follow existing patterns)
- JSON validity check: `serde_json::from_str::<serde_json::Value>(&output).is_ok()`

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

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Branch is `feature/json-tty-detection`.
- Never skip a story or reorder without noting a blocker.
- When changing `Commands` variants (adding fields), search all match arms in `main.rs` and update destructuring patterns — compiler catches this but grep first to be sure.
- When adding fields to Commands variants, check `cli.rs` `is_exempt_from_config_check` method — format commands are not exempt, no change needed there.
- `resolve_format` must be a pure function (accept `tty: bool` param, not call `is_tty()` internally) so it is unit-testable without TTY.
- Do not add new crate dependencies for TTY detection — use `std::io::IsTerminal`.
- Edition 2024: `unsafe` block required for `std::env::set_var` in tests.
