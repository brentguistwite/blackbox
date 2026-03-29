# Ralph Agent Instructions — multi-ai-tracking

You are an autonomous coding agent implementing the `multi-ai-tracking` feature for the `blackbox` CLI. Read prd.json in this directory for the full story list. Implement stories in dependency order. Never commit to `main` — always work on `feature/multi-ai-tracking`.

---

## Your Task

Follow these steps in order:

1. **Read prd.json** — understand all stories, priorities, deps before writing any code.
2. **Create feature branch**: `git checkout -b feature/multi-ai-tracking`
3. **Implement P0 stories first** (US-001, US-002, US-003, US-004, US-005, US-008, US-011, US-012, US-013, US-017), then P1 (US-006, US-007, US-009, US-010, US-014, US-015, US-016).
4. **Within each story**: write the test first (TDD), then implement until the test passes.
5. **After each story**: `cargo build` must pass before moving to the next story.
6. **After all stories**: run `cargo test` and `cargo clippy` and fix all failures/warnings.
7. **Do not fix tests after every iteration** during rapid implementation — batch test fixes at the end of each priority tier, or when explicitly asked.
8. **Write a progress report** after completing each story (see format below).
9. **Do not push** unless the user asks.

---

## Story Dependencies

```
US-017 (serde_yaml dep) — no deps, do first

US-001 (AiToolDetector trait)
  ├─ US-002 (DB: tool param on insert)
  │    ├─ US-004 (Codex detector)
  │    │    └─ US-012 (test: Codex parsing)
  │    ├─ US-005 (Copilot detector)
  │    │    └─ US-013 (test: Copilot parsing)
  │    ├─ US-006 (Cursor detector)
  │    │    └─ US-014 (test: Cursor parsing)
  │    ├─ US-007 (Windsurf detector)
  │    ├─ US-008 (DB: query by tool)
  │    ├─ US-009 (output: tool field)
  │    │    └─ US-016 (integration test: multi-tool JSON output)
  │    └─ US-010 (query: tool in AiSession)
  │         └─ US-016
  └─ US-011 (test: trait + ClaudeDetector)

US-003 (process inspection)
  ├─ US-004
  ├─ US-005
  ├─ US-006
  ├─ US-007
  └─ US-015 (test: process detection)
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

- `src/claude_tracking.rs` — the existing Claude session tracking module. This is the template for all new detectors. Read it fully before writing any detector code.
- `src/db.rs` — all DB functions. Modify `insert_ai_session` to accept `tool` param. Add `get_active_sessions_by_tool`.
- `src/poller.rs` — `full_scan()` calls `claude_tracking::poll_claude_sessions`. Replace with `ai_tracking::poll_all_ai_sessions`.
- `src/lib.rs` — add `pub mod ai_tracking;` here.
- `src/output.rs` — JsonAiSession struct. Add `tool: String` field.
- `src/query.rs` — AiSession struct. Add `tool: String` field. SELECT must include tool column.
- `tests/` — look at existing test files for DB setup patterns (`setup_db()`, tempfile, assert_cmd).

### Existing Claude tracking pattern (template for all detectors)

`src/claude_tracking.rs` flow:
1. Read session files from `~/.claude/sessions/*.json` — each is a `SessionFile { pid, session_id, cwd, started_at }`.
2. Phase 1: for each session file, call `insert_ai_session` (INSERT OR IGNORE). Map cwd → watched repo via `map_to_repo`.
3. Phase 2: query DB for all sessions with `ended_at IS NULL`. For each, check if PID is still running via `nix::sys::signal::kill(..., None)`. If not running → `update_session_ended`.
4. Turn count: count non-empty lines in `~/.claude/projects/{encoded_cwd}/{session_id}.jsonl`.

All new detectors follow this same two-phase pattern. The difference is how each tool stores its session state.

### DB schema — ai_sessions table

```sql
CREATE TABLE IF NOT EXISTS ai_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tool TEXT NOT NULL DEFAULT 'claude-code',
    repo_path TEXT NOT NULL,
    session_id TEXT NOT NULL UNIQUE,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    turns INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

`tool` column already exists (migration 4). Default is 'claude-code'. No new migration needed — just start writing the tool value explicitly.

### New module: src/ai_tracking.rs

Create this file. It contains:
- `pub trait AiToolDetector` — the shared interface
- `pub struct ClaudeDetector` — wraps claude_tracking::poll_claude_sessions_with_paths
- `pub struct CodexDetector`
- `pub struct CopilotDetector`
- `pub struct CursorDetector`
- `pub struct WindsurfDetector`
- `pub fn poll_all_ai_sessions(conn: &Connection, watched_repos: &[PathBuf])`
- `fn processes_matching(name_pattern: &str) -> Vec<u32>` (private)
- `fn is_any_process_running(name_pattern: &str) -> bool` (private)
- `fn map_to_repo(cwd: &str, watched_repos: &[PathBuf]) -> String` — copy from claude_tracking.rs

### AiToolDetector trait

```rust
pub trait AiToolDetector {
    fn tool_name(&self) -> &'static str;
    fn poll(&self, conn: &rusqlite::Connection, watched_repos: &[std::path::PathBuf]);
}
```

### poll_all_ai_sessions

```rust
pub fn poll_all_ai_sessions(conn: &rusqlite::Connection, watched_repos: &[std::path::PathBuf]) {
    let detectors: Vec<Box<dyn AiToolDetector>> = vec![
        Box::new(ClaudeDetector::default()),
        Box::new(CodexDetector::default()),
        Box::new(CopilotDetector::default()),
        Box::new(CursorDetector::default()),
        Box::new(WindsurfDetector::default()),
    ];
    for detector in &detectors {
        detector.poll(conn, watched_repos);
    }
}
```

### process inspection: pgrep approach

```rust
fn processes_matching(name_pattern: &str) -> Vec<u32> {
    use std::process::Command;
    use std::time::Duration;
    // pgrep -i returns newline-separated PIDs
    let output = std::thread::spawn(move || {
        Command::new("pgrep")
            .args(["-i", name_pattern])
            .output()
    });
    // 2s timeout via recv_timeout pattern (or just unwrap with a fallback)
    match output.join() {
        Ok(Ok(out)) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| l.trim().parse::<u32>().ok())
                .collect()
        }
        _ => vec![],
    }
}
```

Do not use the `sysinfo` crate — `pgrep` subprocess is sufficient and keeps dependencies minimal.

### Codex session meta parsing

```rust
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexSessionMeta {
    cwd: String,
    created_at: String,
    updated_at: String,
}
```

Session file path: `~/.codex/sessions/2024/03/15/rollout-abc123.jsonl`
Session ID: `2024-03-15-rollout-abc123` (year-month-day from path + filename stem).
Turns: count all non-empty lines in the JSONL file.
Parse `updated_at` as ISO 8601 → check if older than 5 minutes for ended detection.

### Copilot workspace.yaml schema

Minimal YAML to parse:
```yaml
cwd: /Users/foo/bar/project
```

Use `serde_yaml::from_str::<serde_json::Value>` if the YAML shape is unclear, then extract `cwd` as string. Or define a typed struct:
```rust
#[derive(serde::Deserialize)]
struct CopilotWorkspace {
    cwd: Option<String>,
}
```

### Cursor workspace.json schema

```json
{"folder": "/Users/foo/project"}
```

Or for remote workspaces (skip these):
```json
{"folder": "vscode-remote://ssh-remote+myhost/path/to/project"}
```

Skip any folder value that starts with a URI scheme (`contains("://")` check is sufficient).

### Windsurf fallback: synthetic sessions

When Windsurf data dir is absent, use process-only detection:

```rust
// Query repos with recent git activity
let recent_repos: Vec<String> = conn.prepare(
    "SELECT DISTINCT repo_path FROM git_activity WHERE timestamp > ?1"
)?.query_map([&cutoff_rfc3339], |r| r.get(0))?
  .filter_map(|r| r.ok()).collect();
```

Synthetic session ID: `windsurf-{repo_slug}-{today_date}` where repo_slug = last path component of repo_path.

### map_to_repo function

Copy the existing `map_to_repo` from claude_tracking.rs into ai_tracking.rs as a private helper. Remove it from claude_tracking.rs and have ClaudeDetector call the one in ai_tracking.rs, or just duplicate it (both are small, duplication is fine here).

### Cargo.toml: serde_yaml dependency

```toml
serde_yaml = "0.9"
```

Add under `[dependencies]`. Note: serde_yaml 0.9 uses serde 1.x which is already in the project.

---

## Edge Cases

- **Tool not installed**: sessions dir/file doesn't exist → detector returns immediately without error. Log at `debug` level: `"{tool} data dir not found, skipping"`.
- **Permission denied reading session files**: `std::fs::read_dir` returns `Err` → skip that file/dir, log at `debug`.
- **Malformed JSONL first line**: serde_json parse fails → skip that session file, no panic.
- **Malformed workspace.yaml**: serde_yaml parse fails → skip that session dir, no panic.
- **vscode-remote:// URIs in Cursor workspace.json**: filter out any folder containing `"://"`.
- **pgrep not available**: `Command::new("pgrep").output()` returns `Err` → processes_matching returns `[]`, is_any_process_running returns `false`. Cursor/Windsurf/Copilot sessions will appear ended immediately on Linux systems without pgrep (rare but possible).
- **Linux platform**: Cursor path is `~/.config/Cursor/User/workspaceStorage/`. Use `#[cfg(target_os = "macos")]` / `#[cfg(not(target_os = "macos"))]` to select path.
- **Windsurf data dir present but empty**: iterate zero workspace entries, no sessions inserted.
- **DB session for a tool that is no longer installed**: get_active_sessions_by_tool returns those sessions; since the process won't be running, they'll be marked ended on next poll. This is correct behavior.
- **Rust 2024**: `unsafe` block required for `std::env::set_var` in tests.
- **Duplicate session IDs across tools**: schema has `session_id TEXT NOT NULL UNIQUE`. Session IDs must be tool-scoped to avoid collisions. Prefix Cursor sessions with `cursor-`, Windsurf synthetic sessions with `windsurf-`, Copilot sessions use UUID (already unique), Codex sessions use `{date}-{stem}` (unlikely to collide).
- **Testability**: all detectors take optional path overrides (same `Option<&Path>` pattern as `poll_claude_sessions_with_paths`) so tests can inject temp dirs without touching `~`.

---

## New Files

### src/ai_tracking.rs

Main module for this feature. Contains trait, all detector structs and impls, process inspection helpers, `poll_all_ai_sessions`. Register as `pub mod ai_tracking;` in `src/lib.rs`.

### tests/ai_tracking_test.rs

Integration tests for US-011 through US-015. Use tempfile for temp dirs. Use the `setup_db()` pattern from existing test files (e.g., query_test.rs or db_test.rs if present). All detectors must accept path overrides via constructor args or with_paths variants.

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
The final story (US-018) requires updating README.md (Commands table, feature descriptions) and AGENTS.md (Architecture tree, new modules, key dependencies). These are in the project root. Read them first, then make minimal targeted edits — don't rewrite entire files.

## CRITICAL: After Each Story

After committing, you MUST do these two things:

1. **Update prd.json** — set `"passes": true` for the completed story. This is how ralph knows to move to the next story. Do NOT commit this change (prd.json is in .ralph/ which is gitignored).
2. **Do NOT fix clippy warnings in files unrelated to the current story.** Only touch files required by the story's acceptance criteria.

## Rules

- Never commit directly to `main`. Feature branch only.
- Never use `git add -A` or `git add .` — stage specific files by name.
- Do not push unless the user says so.
- All new public types/fns must be `pub`.
- Do not add `sysinfo` or other new crates for process detection — use `pgrep` subprocess.
- Do not add `async` — entire codebase is synchronous blocking (daemon runs in thread, no tokio in this context).
- Keep ai_tracking.rs as one file unless it exceeds ~600 lines; do not split into sub-modules prematurely.
- No new DB migrations — tool column already exists in ai_sessions table.
- Log at `debug` for "not found" paths, `warn` for unexpected errors.
- When adding struct fields (AiSession, JsonAiSession), always search for all construction sites and update them — the compiler will catch missed cases.
