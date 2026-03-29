# Blackbox Improvements Plan

Research completed 2026-03-25. 18 improvements organized by tier.

---

## Tier 1: High Impact, Leverages Existing Data

### 1. Wire up directory presence into time estimation
Data already collected via shell hooks into `directory_presence` table. `query_presence()` exists but is never called. Connecting it to `estimate_time_v2` would make time estimates significantly more accurate — know when someone was *in* a repo, not just when they committed.

### 2. Contribution heatmap (GitHub green squares, local)
Cross-repo activity calendar in terminal. git-stats does this per-repo; nobody does it across all repos combined for private work. High emotional resonance. Could be `blackbox heatmap` or part of `week`/`month` output.

### 3. Work rhythm insights
From commit timestamps already stored:
- Peak productive hours (commits-per-hour-of-day histogram)
- Day-of-week patterns
- After-hours / weekend ratio (sustainability signal)
- Session length distribution (flow proxy)
- Burst vs. steady commit patterns

Framing: "mirror, not a score." Research consistently shows devs welcome self-reflection metrics but reject evaluation metrics.

### 4. PR cycle time tracking
`gh` integration exists. Track: time-to-first-review, total cycle time (first commit -> merge), PR size (lines changed), iteration count. Currently PR info is fetched live at query time and not persisted — needs to be stored in DB at poll time.

### 5. Code churn rate
% of your own lines reworked/reverted within N days. GitClear's data shows this metric rising industry-wide. Detectable from git history already stored.

---

## Tier 2: Medium Effort, Strong Differentiation

### 6. `blackbox insights` — LLM-powered behavioral analysis
Feed week/month of activity data to LLM for pattern detection:
- "Your PRs that merged fastest were under 200 lines"
- "You tend to do deep work on Tuesdays"
- "Your commit messages got shorter toward end of week"
- "3x more bug-fix commits after Friday afternoon pushes"

Enterprise tools (Waydev, LinearB, Swarmia) charge $$$$ for this. Nobody offers it as a local CLI.

### 7. `blackbox perf-review` — self-assessment generator
LLM summarizes repos touched, features shipped, PRs reviewed, code themes, time invested over a quarter. Framed for performance review season. Strong latent demand.

### 8. Multi-AI-tool tracking
Schema already supports `tool` field (hardcoded to `claude-code`). Add detection for:

| Tool | Detection Path | Format |
|---|---|---|
| Codex CLI | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | JSONL, `SessionMeta` has `cwd`, `createdAt`/`updatedAt` |
| Copilot CLI | `~/.copilot/session-state/<uuid>/workspace.yaml` + `events.jsonl` | YAML + JSONL, `cwd` in workspace.yaml |
| Cursor | `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb` | SQLite, `cwd` via `workspaceStorage/<hash>/workspace.json` |
| Windsurf | `~/Library/Application Support/Windsurf/` | Poorly documented, process detection more reliable |

None write PID files — "still running?" detection needs process inspection (like existing Claude session tracking).

### 9. Context-switch frequency metric
Branch-switching already tracked. Surface: "You switched branches 14 times today across 5 repos" as a distraction signal. Research: 23 min to regain focus after each switch. Novel metric nobody else tracks.

### 10. Weekly digest
`blackbox digest` — structured weekly summary. WakaTime's weekly email is consistently cited as their killer feature. Could optionally deliver via OS notification or write to file.

---

## Tier 3: UX & Polish

### 11. `--json` on every command + TTY auto-detection
Strip ANSI and output JSON/CSV automatically when piped. Enables `blackbox today --json | jq '.repos[] | .name'`.

### 12. Richer `status` command
Show: PID, uptime, last-poll timestamp, repos watched count, DB size, event count today. Answer "is it working?" without reading logs.

### 13. SIGHUP config reload
Reload config without daemon restart. Standard daemon convention.

### 14. OS notifications (opt-in)
`notify-rust` crate. Rate-limited, opt-in via config. E.g., daily summary notification.

### 15. Next-step suggestions in output
After `blackbox today`, suggest `blackbox today --summarize` or `blackbox live`. Like `git status` suggests `git add`.

### 16. Commit message quality tracking
Score commit messages over time. Flag vague ones ("fix stuff", "wip", "updates"). Trend over weeks. Correlate with later reverts.

### 17. Streak as ambient data
"12-day streak" shown in `today` output. Never nag about broken streaks. Just show what you *have* done.

### 18. `blackbox repo <path>` — single-repo deep dive
onefetch-inspired: language breakdown, top files changed, time invested, PR history, branch activity for one repo.

---

## Competitive Context

| | Blackbox | WakaTime | git-standup | git-hours | ActivityWatch |
|---|---|---|---|---|---|
| Passive/daemon | Yes | Yes (IDE) | No | No | Yes |
| Multi-repo | Yes | Yes | Yes | No | No |
| Local/offline | Yes | No (cloud) | Yes | Yes | Yes |
| Git-native | Yes | No | Yes | Yes | No |
| Time estimation | Partial | Yes | No | Yes | No |
| LLM summaries | Yes | No | No | No | No |
| PR/review tracking | Yes | No | No | No | No |
| AI session tracking | Yes | No | No | No | No |

**Blackbox moat:** daemon + SQLite persistence + git-native + local-first + LLM integration. No competitor combines all of these.

**Identity-defining features** (tell-a-coworker tier): #2 heatmap, #6 insights, #7 perf-review.

## Research Sources

Competitor tools: git-standup, git-hours, git-stats, onefetch, WakaTime, Wakapi, ActivityWatch, Codealike, git-quick-stats
Frameworks: DORA 2024, SPACE (ACM Queue), DevEx (Noda et al. 2023), DX Core 4
UX patterns: clig.dev, lazygit, atuin, starship
AI tools: Gitmore, standup.so, aicommits, Faros AI (AI productivity paradox report)
