# Ralph Agent Instructions — next-step-suggestions

You are an autonomous coding agent implementing the `next-step-suggestions` feature for the blackbox CLI. Work story-by-story, write tests first (TDD), verify each story before moving to the next.

---

## Your Task

Follow these 9 steps for each story:

1. **Read** the story from `prd.json` and identify acceptance criteria.
2. **Write tests first** — add failing tests to the appropriate test file before touching implementation.
3. **Implement** the minimal change to make tests pass.
4. **Run quality checks** (see below) — fix all failures before proceeding.
5. **Verify** acceptance criteria are met one by one.
6. **Report** status (see Progress Report Format).
7. **Commit** to the feature branch with a concise message.
8. **Check** no regressions in existing tests.
9. Repeat for the next story.

Never implement multiple stories simultaneously. Never skip to implementation before tests exist.

---

## Story Dependencies

```
US-001  (no deps)
  ├─ US-002  (ruleset)
  │    └─ US-007  (integration tests — needs US-004 too)
  └─ US-003  (rendering)
       └─ US-004  (wire into run_query)
            ├─ US-005  (non-TTY suppression)
            ├─ US-006  (config opt-out)
            └─ US-007
```

Implement in order: US-001 → US-002 → US-003 → US-004 → US-005 → US-006 → US-007.

---

## Quality Checks

Run these after every story. All must pass before marking done.

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
- `src/lib.rs` — module declarations; add `pub mod suggestions;` here
- `src/cli.rs` — `Commands` enum; Today/Week/Month/Standup variants
- `src/main.rs` — `run_query(period_label, range_fn, format, summarize)` dispatches Today/Week/Month
- `src/output.rs` — `OutputFormat` enum, `render_summary`, `render_json`, `render_csv`, `is_tty()`
- `src/config.rs` — `Config` struct with `#[serde(default)]` pattern; add `show_hints` here
- `src/daemon.rs` — `is_daemon_running(data_dir: &Path) -> anyhow::Result<Option<u32>>`
- `tests/cli_test.rs` — assert_cmd integration tests (existing)
- `tests/suggestions_test.rs` — create new for US-007

**`OutputFormat` enum** (`src/output.rs`):
```rust
#[derive(Clone, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Pretty,
    Json,
    Csv,
}
```

**`run_query` signature** (`src/main.rs`):
```rust
fn run_query(
    period_label: &str,
    range_fn: fn() -> (DateTime<Utc>, DateTime<Utc>),
    format: OutputFormat,
    summarize: bool,
) -> anyhow::Result<()>
```

**Match arm pattern for Today** (same for Week/Month):
```rust
Commands::Today { format, summarize } => {
    run_query("Today", blackbox::query::today_range, format, summarize)?;
}
```

**`is_tty()`** already exists in `src/output.rs` (returns `std::io::stdout().is_terminal()`).

**`Config` struct** (`src/config.rs`): uses `#[serde(default = "fn_name")]` pattern. All fields have defaults. `Config::default()` exists.

---

## Implementation Guide

### US-001: `src/suggestions.rs` — SuggestionContext + generate_suggestions()

Create `src/suggestions.rs`:

```rust
use crate::output::OutputFormat;

#[derive(Debug, Clone, PartialEq)]
pub enum SuggestionCommand {
    Today,
    Week,
    Month,
    Standup,
}

#[derive(Debug, Clone)]
pub struct SuggestionContext {
    pub command: SuggestionCommand,
    pub has_activity: bool,
    pub summarize_used: bool,
    pub daemon_running: bool,
    pub llm_configured: bool,
    pub format: OutputFormat,
}

/// Returns zero or more hint strings based on context.
/// Pure function — no I/O, no side effects.
pub fn generate_suggestions(ctx: &SuggestionContext) -> Vec<String> {
    // Guard: machine-readable formats or summarize path → no hints
    if !matches!(ctx.format, OutputFormat::Pretty) || ctx.summarize_used {
        return vec![];
    }
    // Guard: Standup command → no hints
    if ctx.command == SuggestionCommand::Standup {
        return vec![];
    }
    // ... ruleset in US-002
    vec![]
}
```

Add `pub mod suggestions;` to `src/lib.rs`.

### US-002: Per-command ruleset inside generate_suggestions()

After the guards from US-001, implement:

```rust
let cmd_name = match ctx.command {
    SuggestionCommand::Today => "today",
    SuggestionCommand::Week  => "week",
    SuggestionCommand::Month => "month",
    SuggestionCommand::Standup => unreachable!(),
};

let mut hints: Vec<String> = Vec::new();

// Highest priority: daemon not running
if !ctx.daemon_running {
    hints.push("blackbox start".to_string());
}

if ctx.daemon_running && !ctx.has_activity {
    hints.push("blackbox doctor".to_string());
}

if ctx.has_activity && ctx.llm_configured {
    hints.push(format!("blackbox {} --summarize", cmd_name));
}

if ctx.has_activity {
    hints.push("blackbox live".to_string());
}

// Cap at 3 to avoid noise
hints.truncate(3);
hints
```

### US-003: `render_suggestions()` in output.rs

```rust
/// Render hint lines as dim+italic text. Returns "" when hints is empty.
pub fn render_suggestions(hints: &[String]) -> String {
    if hints.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n");
    for hint in hints {
        let line = format!("  hint: {}", hint).dimmed().italic().to_string();
        out.push_str(&line);
        out.push('\n');
    }
    out
}
```

Testing: call `colored::control::set_override(false)` in the test to strip ANSI before asserting on content.

### US-004: Wire into run_query

In `run_query` in `main.rs`, modify the `OutputFormat::Pretty` branch:

```rust
OutputFormat::Pretty => {
    blackbox::output::render_summary(&summary);
    if blackbox::output::is_tty() {
        let ctx = blackbox::suggestions::SuggestionContext {
            command: /* map period_label or pass as param */,
            has_activity: summary.total_commits > 0 || summary.total_reviews > 0,
            summarize_used: summarize,
            daemon_running: blackbox::daemon::is_daemon_running(&data_dir)
                .unwrap_or(None)
                .is_some(),
            llm_configured: config.llm_provider.is_some(),
            format: format.clone(),
        };
        let hints = blackbox::suggestions::generate_suggestions(&ctx);
        let rendered = blackbox::output::render_suggestions(&hints);
        if !rendered.is_empty() {
            eprint!("{}", rendered);
        }
    }
}
```

**Note:** `run_query` currently takes `period_label: &str`. To map it to `SuggestionCommand`, either: (a) add a `SuggestionCommand` param to `run_query`, or (b) derive it from `period_label` with a match. Option (a) is cleaner — update all three call sites in `main.rs`.

**`run_query` needs access to `config` and `data_dir` for daemon check.** Currently `run_query` loads config and data_dir internally. No refactor needed — just use the already-loaded `config` and `data_dir` variables inside `run_query`.

Hints printed with `eprint!`/`eprintln!` (stderr), not `print!` (stdout).

### US-005: Non-TTY suppression

The `is_tty()` gate in US-004 handles this. Explicitly document in code:

```rust
// Only emit hints in interactive TTY sessions.
// Suppresses hints when piped (blackbox today | jq ...) or in CI.
if blackbox::output::is_tty() {
    // ... hint generation
}
```

No additional logic needed beyond the gate added in US-004.

### US-006: Config opt-out

In `src/config.rs`:

```rust
fn default_show_hints() -> bool { true }

pub struct Config {
    // ... existing fields ...
    #[serde(default = "default_show_hints")]
    pub show_hints: bool,
}
```

Update `Config::default()`:
```rust
show_hints: default_show_hints(),
```

In `run_query` in `main.rs`, add a second gate:
```rust
if blackbox::output::is_tty() && config.show_hints {
    // ... hint generation
}
```

When adding `show_hints` to `Config`, search all test files for `Config {` struct literal construction and add `show_hints: true` to each. Compiler catches missing fields — fix all.

### US-007: Integration tests

`tests/suggestions_test.rs` — pure unit tests for `generate_suggestions`. No DB or CLI needed. Use `blackbox::suggestions::*` and `blackbox::output::OutputFormat`.

`tests/cli_test.rs` — add one CLI test: `blackbox today --format json` stdout is valid JSON with no hint text. The hint goes to stderr and is suppressed in non-TTY anyway.

---

## Edge Cases

- **JSON/CSV format:** `generate_suggestions` returns `[]` immediately — no hints ever contaminate machine-readable output. The TTY gate in `run_query` provides a second layer.
- **`--summarize` flag:** When `summarize == true`, `run_query` takes the LLM path entirely (skips `render_summary`). Set `summarize_used: true` in context. `generate_suggestions` short-circuits to empty. No hints on LLM output.
- **Daemon not running + has activity:** DB has data from a previous run. Daemon hint fires (`blackbox start`) but `has_activity` may still be true. Daemon hint takes priority (first in vec). Limit to 3 total hints.
- **Daemon check failure:** `is_daemon_running` returns `Err` if PID file is corrupt. Use `.unwrap_or(None).is_some()` — treats error as "not running", which is safe (shows the start hint at worst).
- **Standup command:** Standup already formats output for sharing. Hints would be confusing (user is copying text). Always empty for Standup.
- **`blackbox live` command:** Live TUI has no run_query path — no hint wiring needed.
- **`blackbox standup --format json`:** Standup command in `main.rs` doesn't call `run_query`. If hints were added for standup, they'd need separate wiring. For now: no hints for standup.
- **Non-TTY stderr:** Even though hints go to stderr, we gate on `is_tty()`. This prevents hints appearing in CI log output where stderr is captured. If a user pipes stdout but reads stderr manually, they get no hints — acceptable behavior.
- **`show_hints = false` in config:** Takes precedence over TTY check. Check `config.show_hints` before `is_tty()` to short-circuit cheaply.
- **assert_cmd tests spawn subprocesses:** Subprocess stdout is a pipe → `is_tty()` returns false → hints suppressed automatically. Existing tests are unaffected.
- **Colored crate + set_override:** `render_suggestions` uses `.dimmed().italic()`. In tests, call `colored::control::set_override(false)` before asserting string content, then reset with `colored::control::unset_override()`.

---

## Test File Guidance

- `tests/suggestions_test.rs` (new): unit tests for `generate_suggestions` with all combinations
- `tests/cli_test.rs` (existing): add JSON-clean-stdout test
- `tests/config_test.rs` (existing or create): test `show_hints` serde default
- Rust 2024: `unsafe` block required for `std::env::set_var` in tests
- `assert_cmd::Command::cargo_bin("blackbox")` for CLI tests
- JSON validation: `serde_json::from_str::<serde_json::Value>(&stdout).is_ok()`

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
The final story requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Branch is `feature/next-step-suggestions`.
- Never skip a story or reorder without noting a blocker.
- `generate_suggestions` must be pure (no I/O). All context passed via `SuggestionContext`.
- Hints go to **stderr** (`eprint!`), never stdout. Stdout must remain clean for piping.
- When adding `show_hints` to `Config`, update ALL `Config { .. }` struct literals in tests — compiler catches missing fields.
- Do not add new crate dependencies. `colored` is already in `Cargo.toml`.
- Edition 2024: `unsafe` block required for `std::env::set_var` in tests.
- Keep hints minimal (max 3) and command-only — no prose explanations in the hint strings themselves.
