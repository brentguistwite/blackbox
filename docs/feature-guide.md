# Blackbox Feature Guide

All features added by the ralph-conductor batch (plans 01–18). 876 tests, all passing.

**README status:** `digest` command is missing from README. Everything else is documented.

---

## 01 — Directory Presence Time Tracking

**Problem:** Time estimation relied solely on gaps between commits. If you spent 45 minutes reading code before your first commit, that time was invisible — you'd get a flat 30-minute "first commit credit" instead.

**What it does:** Shell hooks (`blackbox hook zsh/bash/fish`) record when you `cd` into and out of git repos. The time estimator now anchors session start times to when you actually entered the directory, not when you first committed.

**How to test:**
```bash
# Install the shell hook
eval "$(blackbox hook zsh)"

# cd into a tracked repo, wait a bit, make a commit
cd ~/some-tracked-repo
# ... work for a few minutes ...
git commit --allow-empty -m "test presence tracking"

# Check that estimated time reflects actual presence, not just the 30m default
blackbox today
```

**Automated tests:** `tests/time_test.rs`, `tests/hook_test.rs`

---

## 02 — Contribution Heatmap

**Problem:** GitHub's contribution graph only shows one account's public repos. No way to see your full commit activity across all repos (work, personal, private) in one view.

**What it does:** `blackbox heatmap` renders a GitHub-style green-squares grid in the terminal. Aggregates commit counts per calendar day across every tracked repo. Color intensity scales with commit volume. Shows total commits, active days, and longest streak.

**How to test:**
```bash
blackbox heatmap              # default: last 52 weeks
blackbox heatmap --weeks 12   # last 12 weeks
```

**Automated tests:** `tests/heatmap.rs`

---

## 03 — Work Rhythm Analysis

**Problem:** Developers have no data-driven view of *when* they work best. Are you a morning coder? Do you work more on Fridays? Is your commit pattern bursty or steady? Without this mirror, you can't make informed decisions about schedule optimization.

**What it does:** Analyzes commit timestamps to surface patterns: hour-of-day histogram, day-of-week distribution, after-hours/weekend ratio, session length stats (median/p90/mean), and burstiness (coefficient of variation of inter-commit gaps).

**How to test:**
```bash
blackbox rhythm               # last 30 days
blackbox rhythm --days 7      # last week
blackbox rhythm --format json # machine-readable
```

**Automated tests:** `tests/rhythm_test.rs`

---

## 04 — PR Cycle Time Metrics

**Problem:** "How long do my PRs take to get reviewed and merged?" is unanswerable without manually checking each one. Slow review cycles are invisible until they become a pattern.

**What it does:** Collects PR snapshots via `gh` CLI during each daemon poll. Surfaces: cycle time (created → merged), time to first review, PR size (additions + deletions), iteration count (changes-requested reviews). Data persists in `pr_snapshots` table.

**How to test:**
```bash
# Requires gh CLI authenticated
blackbox prs                  # last 30 days
blackbox prs --days 7
blackbox prs --repo /path/to/specific/repo
blackbox prs --format json
```

**Automated tests:** `tests/pr_cycle_test.rs`, `tests/enrichment_test.rs`

---

## 05 — Code Churn Rate

**Problem:** High code churn (writing code then quickly rewriting it) can signal unclear requirements or premature implementation, but it's invisible without measurement. You can't improve what you can't see.

**What it does:** Measures how much recently written code gets reworked within a configurable window. Uses the GitClear heuristic: for each pair of commits touching the same file within the window, `min(lines_added_by_A, lines_deleted_by_B)` = churned lines. >25% = possible concern, <10% = stable work.

**How to test:**
```bash
blackbox churn                # 14-day window default
blackbox churn --window 7     # 7-day window
blackbox churn --repo /path/to/repo
blackbox churn --format json
blackbox churn --format csv
```

**Automated tests:** `tests/churn_test.rs`

---

## 06 — LLM-Powered Insights

**Problem:** Raw activity data (commit counts, time estimates) tells you *what* happened but not *what it means*. Patterns like "you always fix bugs on Fridays" or "your PR review turnaround dropped this week" require manual analysis.

**What it does:** Aggregates a week or month of activity data (commit distribution by day/hour, bug-fix ratio, message length trends, PR merge times, per-repo breakdown) and sends it to an LLM for behavioral analysis. `--format json` skips the LLM and emits raw stats.

**How to test:**
```bash
# Requires llm_api_key in config
blackbox insights                 # this week, streamed
blackbox insights --window month
blackbox insights --format json   # raw data only, no LLM call
```

**Automated tests:** `tests/insights_test.rs`

---

## 07 — Performance Review Self-Assessment

**Problem:** Performance review season means scrambling to remember what you did over the last quarter. Commit logs exist but synthesizing them into a coherent narrative is tedious and time-consuming.

**What it does:** Aggregates a quarter (or custom date range) of git activity, PR contributions, code review history, and AI tool usage. Extracts recurring themes from commit messages. Feeds structured context to an LLM which produces a markdown self-assessment with sections: Summary, Key Contributions, Technical Themes, Collaboration & Code Review, Time Investment.

**How to test:**
```bash
# Requires llm_api_key in config
blackbox perf-review                                    # current quarter
blackbox perf-review --from 2026-01-01 --to 2026-03-31 # custom range
```

**Automated tests:** `tests/perf_review_test.rs` (assert_cmd: API key error, empty range, invalid date)

---

## 08 — Multi-AI Tool Tracking

**Problem:** Developers use multiple AI coding tools (Claude Code, Codex, Copilot, Cursor, Windsurf) but have no unified view of AI-assisted vs manual work. Session data is scattered across tool-specific directories.

**What it does:** Passively detects and tracks sessions from all five AI tools. Each tool has a custom detector that reads its session files and/or checks running processes. Sessions appear in `today`, `week`, `month`, and `standup` output with a `tool` field. Tools that aren't installed are silently skipped.

**How to test:**
```bash
# If you use any AI coding tools, their sessions should appear automatically
blackbox today              # look for tool-annotated sessions
blackbox today --format json | jq '.ai_sessions'
```

**Automated tests:** `tests/ai_tracking_test.rs`, `tests/ai_tracking_integration_test.rs`, `tests/codex_detector_test.rs`, `tests/claude_tracking_test.rs`

---

## 09 — Context-Switch Frequency

**Problem:** Context switching has a real cognitive cost (~23 min per switch per Gloria Mark's research), but developers have no visibility into how often they switch. "I was productive today" might mean "I switched branches 14 times across 5 repos."

**What it does:** Tracks branch switches from `git_activity` and estimates focus cost. Filters noise: detached HEAD states, same-branch re-checkouts, rapid round-trips (A→B→A within 2 min). Surfaces per-repo switch counts and total focus cost in pretty/json/csv output. `blackbox focus` gives a dedicated focus report. `standup` flags high switch counts (≥5).

**How to test:**
```bash
blackbox focus              # today's focus report
blackbox focus --week       # this week
blackbox today              # context switches appear in activity output
blackbox today --format json | jq '.context_switches'
```

**Automated tests:** `tests/cli_test.rs` (focus-related tests)

---

## 10 — Weekly Digest

**Problem:** Standups are daily snapshots. There's no structured weekly rollup that aggregates git, PR, review, and AI session data with week-over-week comparison — useful for 1:1s, sprint retros, or personal tracking.

**What it does:** Aggregates a full week of activity into a formatted summary. Includes commit counts, time estimates, PR activity, review contributions, AI sessions, and week-over-week deltas. Can write to file, send OS notification, or pipe as JSON/CSV.

**How to test:**
```bash
blackbox digest                       # current week
blackbox digest --week -1             # last week
blackbox digest --format json
blackbox digest --format csv
blackbox digest --output-file weekly.md
blackbox digest --compare             # week-over-week comparison
blackbox digest --notify              # send OS notification
```

**Automated tests:** `tests/digest_test.rs`

---

## 11 — JSON/CSV Output & TTY Auto-Detection

**Problem:** CLI output was pretty-only. Piping `blackbox today` into `jq` or a spreadsheet required scraping ANSI-colored text. No machine-readable output path.

**What it does:** Adds `--json`, `--csv`, and `--format` flags to all output commands. When stdout is piped or redirected (not a TTY), output auto-defaults to JSON. ANSI color codes are stripped in non-TTY mode. Explicit flags always take precedence over auto-detection.

**How to test:**
```bash
blackbox today --json
blackbox today --csv
blackbox today --format json | jq .     # structured output
blackbox today | cat                    # piped = auto-JSON
blackbox rhythm --format json
blackbox prs --format json
```

**Automated tests:** `tests/output_test.rs`, `tests/output.rs`, `tests/cli_test.rs`

---

## 12 — Enhanced Status Command

**Problem:** "Is the daemon working?" required reading logs or checking process tables. No quick answer to "when did it last poll? how many repos is it watching? how big is the DB?"

**What it does:** `blackbox status` shows a health indicator (green/yellow/red), PID, uptime, last poll time, repos watched, DB size, and today's event count. Works without a running daemon (shows last-known state). Works without a config file (fresh install shows `Stopped`).

**How to test:**
```bash
blackbox status               # pretty output with health indicator
blackbox status --format json # machine-readable
# With daemon running: should show ✓ Running with live stats
# Without daemon: should show ✗ Stopped with last-known state
```

**Automated tests:** `tests/daemon_test.rs`

---

## 13 — SIGHUP Config Reload

**Problem:** Changing config (adding watch dirs, adjusting poll interval) required stopping and restarting the daemon. Disruptive if the daemon is mid-poll.

**What it does:** `blackbox reload` sends SIGHUP to the running daemon. The daemon re-reads `config.toml` and applies changes without restarting. If the config file is missing or invalid, it logs a warning and keeps the previous config.

**How to test:**
```bash
blackbox start
# Edit ~/.config/blackbox/config.toml (e.g., add a watch dir)
blackbox reload
blackbox status   # should reflect new config
```

**Automated tests:** Signal handler tests in the daemon module

---

## 14 — OS Desktop Notifications

**Problem:** You have to remember to run `blackbox today` to see your activity. No passive nudge at end-of-day to review what you accomplished.

**What it does:** Sends an OS desktop notification with your daily activity summary at a configurable time. Shows commit count, repo count, and estimated time. Rate-limited to once per day via `notification_log` DB table (daemon restarts won't re-fire). Skips headless/SSH sessions. No notification if no commits today.

**How to test:**
```bash
# Enable in config:
#   notifications_enabled = true
#   notification_time = "17:00"

blackbox start   # daemon checks the clock each poll cycle
# At 17:00, you should see an OS notification

# Or test the digest notification path:
blackbox digest --notify
```

**Automated tests:** `tests/config_test.rs`, `tests/db_test.rs` (notification_log)

---

## 15 — Next-Step Suggestions

**Problem:** New users don't know what to do after `blackbox today`. Power users forget about features like `--summarize` or `blackbox live`. No discoverability path.

**What it does:** After `today`, `week`, and `month` pretty output, prints dim/italic hint lines on stderr suggesting logical next commands — like git's "use git push to publish your local commits" hints. Context-aware: suggests `blackbox start` if daemon isn't running, `--summarize` if LLM is configured, `blackbox live` for TUI. Suppressed in JSON/CSV, non-TTY, `--summarize` mode, and for `standup`. Disable with `show_hints = false`.

**How to test:**
```bash
blackbox today            # look for dim hint lines below output
blackbox today --json     # hints should NOT appear
blackbox today | cat      # piped = no hints
# Set show_hints = false in config, verify hints disappear
```

**Automated tests:** `tests/suggestions_test.rs`

---

## 16 — Commit Message Quality

**Problem:** "wip", "fix", "update" — vague commit messages accumulate silently. No feedback loop to improve message quality over time, and no way to correlate low-quality messages with downstream problems like reverts.

**What it does:** Scores every commit message 0–100 during daemon polling. Factors: subject length, conventional commit prefix, body presence, vague pattern detection. Tracks weekly trends and correlates low-quality commits with reverts. Backfills existing commits (up to 200) on first poll.

**How to test:**
```bash
blackbox commit-quality                    # last 8 weeks
blackbox commit-quality --weeks 4
blackbox commit-quality --show-reverts     # revert correlation
blackbox commit-quality --format json
blackbox commit-quality --format csv
```

**Automated tests:** `tests/commit_quality.rs`, `tests/commit_quality_test.rs` (33+ tests: scoring variants, vague patterns, DB dedup, multi-week trend, revert correlation, CLI integration)

---

## 17 — Commit Streak (Ambient)

**Problem:** Streak trackers are usually gamified and guilt-inducing ("You broke your streak!"). Developers want to see their streak as neutral ambient data, not a pressure mechanism.

**What it does:** Shows current consecutive-day commit streak on the `today` summary line. Positive framing only — streak of 0 shows nothing (no "0-day streak" shame). If you haven't committed today, yesterday's streak is still alive until end of day. `streak_exclude_weekends = true` lets weekends pass without breaking the streak.

**How to test:**
```bash
blackbox today   # look for "N-day streak" on the summary line
# Make commits on consecutive days, verify streak increments
# Set streak_exclude_weekends = true, skip a weekend, verify streak holds
```

**Automated tests:** `tests/streak_test.rs`, `tests/streak_integration_test.rs`

---

## 18 — Repo Deep Dive

**Problem:** `blackbox today` shows cross-repo summaries. Sometimes you want to zoom into one specific repo — language breakdown, most-changed files, total time invested, branch activity, PR history — without leaving the terminal.

**What it does:** `blackbox repo <path>` gives an onefetch-inspired single-repo analysis. Language breakdown via git2 tree walk, most-changed files, all-time estimated time, branch activity, and PR history from DB. Works on any git repo; untracked repos show git-derived data with an "untracked" indicator.

**How to test:**
```bash
blackbox repo .                    # current repo
blackbox repo ~/some-other-repo
blackbox repo . --format json      # machine-readable
```

**Automated tests:** `tests/repo_deep_dive_test.rs`
