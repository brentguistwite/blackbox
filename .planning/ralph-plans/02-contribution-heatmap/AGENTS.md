# Ralph Agent Instructions — contribution-heatmap

You are an autonomous coding agent implementing the `blackbox heatmap` feature per the prd.json in this directory. Work story-by-story. Do not skip ahead. Do not modify stories outside the current one's scope.

---

## Your Task

Follow these steps for every story:

1. **Read prd.json** — load the story, its acceptance criteria, and its deps.
2. **Check deps are merged** — if a dep story is not yet complete, stop and report.
3. **Create a feature branch** — `git checkout -b feature/contribution-heatmap` (first story only; subsequent stories commit to the same branch).
4. **Read all files you will touch** before editing — never edit blind.
5. **Implement** — follow acceptance criteria exactly. No gold-plating.
6. **Write/update tests** — unit tests inline in the module; integration tests in `tests/heatmap.rs`.
7. **Run quality checks** (see below). Fix all failures before reporting done.
8. **Commit** — `git commit` with message `feat(heatmap): <story-id> <title>`.
9. **Report** — post progress report (see format below).

---

## Story Dependencies

```
US-001  (no deps)
US-002  → US-001
US-003  (no deps)
US-004  → US-002
US-005  → US-003, US-004
US-006  → US-002
US-007  → US-005, US-006
US-008  → US-005
US-009  → US-007
US-010  → US-007
```

Recommended implementation order: US-001, US-003, US-002, US-006, US-004, US-005, US-007, US-008, US-009, US-010

---

## Quality Checks

Run after every story. All must pass before committing:

```bash
cargo build 2>&1
cargo test 2>&1
cargo clippy -- -D warnings 2>&1
```

If clippy is unavailable (`command not found`), skip it and note in report. Fix all `cargo build` and `cargo test` failures — no exceptions.

---

## Project Context

**Crate:** `blackbox-cli` (lib=`blackbox`, bin=`blackbox`). Rust edition 2024.

**Adding the CLI command:**
- Add `Heatmap { #[arg(long, default_value_t = 52)] weeks: u32 }` to `Commands` enum in `src/cli.rs`
- Add match arm `Commands::Heatmap { weeks } => blackbox::heatmap::run_heatmap(weeks)?` in `src/main.rs`

**Module declaration:**
- Add `pub mod heatmap;` to `src/lib.rs`
- Create `src/heatmap.rs`

**DB patterns** (from `src/db.rs`):
- Open with `blackbox::db::open_db(&db_path)` — returns `anyhow::Result<Connection>`
- All timestamps stored as RFC3339 strings in UTC
- `date(timestamp, 'localtime')` converts to local calendar date in SQLite
- Existing tables: `git_activity` (cols: repo_path, event_type, branch, commit_hash, author, message, timestamp), `directory_presence`, `review_activity`, `ai_sessions`

**Query patterns** (from `src/query.rs`):
- Use `BTreeMap` for ordered key-value results
- Use `DateTime::parse_from_rfc3339(&s)?.with_timezone(&Utc)` to parse timestamps
- `Local::now().date_naive()` for today's local date
- `chrono::Duration::days()` for date arithmetic
- `NaiveDate` from `chrono` for calendar dates

**Output patterns** (from `src/output.rs`):
- Use `colored` crate for ANSI output: `"text".truecolor(r,g,b)` or `.on_truecolor(r,g,b)`
- For no-color environments: `colored::control::set_override(false)` (don't set — let the crate detect)
- Print directly to stdout; functions that return `String` are preferred for testability

**Ratatui patterns** (from `src/tui.rs`):
- Import: `use ratatui::{layout::..., style::{Color, Style, Modifier}, text::{Line, Span}, widgets::..., Frame}`
- Colors: `Color::Rgb(r, g, b)` for custom colors, `Color::DarkGray` for inactive cells
- Widget render: `frame.render_widget(widget, area)`
- Test with `ratatui::backend::TestBackend::new(cols, rows)` + `ratatui::Terminal::new(backend)`

**Loading config + DB** (standard pattern from `src/main.rs`):
```rust
let config = blackbox::config::load_config()?;
let data_dir = blackbox::config::data_dir()?;
let db_path = data_dir.join("blackbox.db");
let conn = blackbox::db::open_db(&db_path)?;
```

**Integration test pattern** (from `tests/`):
```rust
use assert_cmd::Command;
use tempfile::TempDir;
// Set BLACKBOX_DATA_DIR env var to tempdir path
// Insert test data via rusqlite directly
// Run Command::cargo_bin("blackbox").unwrap()
```

**Key deps available** (from `Cargo.toml`): chrono 0.4, rusqlite 0.38 (bundled), ratatui 0.29, colored 2.2, anyhow 1.0. Do NOT add new dependencies without user approval.

---

## Edge Cases

- **No commits in range**: render all-dark grid, print "No commits recorded in this period", exit 0.
- **max_count == 0**: `intensity()` must return 0 for all dates — do not divide by zero.
- **Terminal narrower than grid**: truncate weeks displayed to fit (compute `max_weeks = (terminal_width - 4) / 2` where 4=day labels, 2=chars per cell).
- **Single commit ever**: intensity=1, streak=1, active_days=1 — not 0.
- **Heatmap range start before any recorded data**: show empty cells — not an error.
- **`--weeks` out of range (0 or >260)**: return `anyhow::bail!("weeks must be between 1 and 260")`.
- **Rust 2024 edition**: `unsafe` block required around `std::env::set_var` in tests (use `unsafe { std::env::set_var(...) }`).
- **Struct construction in existing tests**: adding fields to shared structs requires updating ALL test sites — compiler will catch these.

---

## Progress Report Format

After each story, output:

```
STORY: <id> <title>
STATUS: done | blocked | partial
FILES_CHANGED: <list of files>
TESTS_ADDED: <count> (<names>)
BUILD: pass | fail
CLIPPY: pass | fail | skipped
NOTES: <any deviations from AC or issues>
```

---

## Documentation Updates
The final story (US-011) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Branch must be `feature/contribution-heatmap`.
- Never skip a story. Never implement story N+1 logic while working on story N.
- Never use `unsafe { std::env::set_var }` outside of `#[cfg(test)]` blocks.
- Never add Cargo dependencies without user approval.
- All public functions must have doc comments (`///`).
- Do not modify existing tests unless a struct field change forces it (compiler-driven only).
- Intensity tiers must exactly match the 5-level scale defined in US-002 AC.
- The `colored` crate (not ratatui) is the renderer for `run_heatmap` stdout output (US-007).
- Ratatui widget (`render_heatmap`) is for future TUI embedding — implement but do not call from `run_heatmap`.
