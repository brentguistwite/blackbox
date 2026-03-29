# blackbox

Flight recorder for your dev day.

## What it does

Blackbox passively tracks your git activity across all your repos -- commits, branch switches, merges -- and estimates time spent per repo using a session-gap algorithm. Zero config after `blackbox init`.

## Install

```
cargo install blackbox-cli
```

Note: build includes bundled SQLite, so compile may take a minute.

## Quick Start

```
blackbox setup
blackbox today
```

## Commands

| Command | Description |
|---------|-------------|
| `init` | Create config interactively (`--watch-dirs`, `--poll-interval` for non-interactive) |
| `setup` | Full interactive onboarding wizard |
| `start` | Start background daemon |
| `stop` | Stop running daemon |
| `status` | Show daemon status (running/stopped) |
| `today` | Show today's git activity (`--json`, `--csv`, `--format pretty\|json\|csv`, `--summarize`) |
| `week` | Show this week's activity (`--json`, `--csv`, `--format pretty\|json\|csv`, `--summarize`) |
| `month` | Show this month's activity (`--json`, `--csv`, `--format pretty\|json\|csv`, `--summarize`) |
| `standup` | Slack/Teams-friendly activity summary (`--week`, `--json`, `--csv`, `--summarize`) |
| `heatmap` | GitHub-style contribution heatmap (`--weeks N`, default 52) |
| `live` | Interactive TUI dashboard |
| `doctor` | Run health checks and report status |
| `install` | Register as OS service (launchd on macOS, systemd on Linux) |
| `uninstall` | Remove OS service registration |
| `hook <shell>` | Print shell hook script for zsh, bash, or fish |
| `rhythm` | Work rhythm analysis (`--days N`, `--format pretty\|json`) |
| `repo <path>` | Single-repo deep dive: language breakdown, top files, time invested, branches, PRs (`--format pretty\|json`) |
| `completions <shell>` | Generate shell completions |

## Shell Hooks

Shell hooks improve time-per-repo accuracy by tracking directory presence between commits.

```bash
# zsh - add to ~/.zshrc
eval "$(blackbox hook zsh)"

# bash - add to ~/.bashrc
eval "$(blackbox hook bash)"

# fish - add to ~/.config/fish/config.fish
blackbox hook fish | source
```

## Config Reference

Config lives at `~/.config/blackbox/config.toml`:

| Field | Default | Description |
|-------|---------|-------------|
| `watch_dirs` | (required) | List of directories to scan for git repos |
| `poll_interval_secs` | `300` | Seconds between daemon polls |
| `session_gap_minutes` | `120` | Minutes of inactivity before new session |
| `first_commit_minutes` | `30` | Time credit for first commit in a session |

## Output Formats

**Pretty** (default in terminal):
```
Today - 3 repos, 12 commits, 4h 30m estimated

  myproject        8 commits   3h 15m
  other-repo       3 commits   1h 00m
  dotfiles         1 commit      15m
```

**JSON:** `blackbox today --json` (or `--format json`) -- structured output with repo details, commit counts, time estimates, and PR info (when gh CLI available).

**CSV:** `blackbox today --csv` (or `--format csv`) -- flat rows suitable for spreadsheets/pipelines.

**TTY auto-detection:** When stdout is piped or redirected (not a terminal), output defaults to JSON automatically. ANSI color codes are stripped in non-TTY mode. Explicit flags (`--json`, `--csv`, `--format`) always take precedence over auto-detection.

**Standup:** `blackbox standup` -- copy-paste-ready summary for Slack/Teams. Use `--week` for weekly and `--summarize` for LLM-generated summaries.

**Heatmap:** `blackbox heatmap` -- GitHub-style contribution grid in your terminal. Shows commit frequency across all tracked repos as color-intensity blocks. Use `--weeks N` (1-260) to control the range. Displays total commits, active days, and longest streak.

The `--summarize` flag is available on `today`, `week`, `month`, and `standup` to generate a natural-language summary of your activity using an LLM.

**Repo deep dive:** `blackbox repo <path>` -- onefetch-inspired single-repo analysis showing language breakdown (via git2 tree walk), most-changed files, all-time estimated time, branch activity, and PR history. Works on any git repo; repos not yet tracked by blackbox show git-derived data with an "untracked" indicator.

## How It Works

A background daemon polls your watched directories for git repos, recording commits, branch switches, and merges to a local SQLite database. The CLI queries this database and estimates time using a session-gap algorithm: commits within `session_gap_minutes` of each other belong to the same work session, and the first commit in each session gets a configurable time credit.

When shell hooks are installed (see above), blackbox also records directory presence — when you enter and leave a repo directory. Presence data anchors git session start times to when you actually started working, rather than relying on estimated credits. This produces more accurate time estimates, especially for repos where you spend significant time before your first commit.

When `gh` CLI is available, output is enriched with PR titles and URLs.

## Work Rhythm

`blackbox rhythm` analyzes your commit timestamps to surface work patterns — a mirror, not a score. Includes:

- **Hour-of-day histogram** — when you commit most (local time)
- **Day-of-week histogram** — weekday vs weekend distribution
- **After-hours/weekend ratio** — commits outside core hours (09:00–18:00)
- **Session length distribution** — median, p90, mean session durations
- **Commit pattern** — bursty vs steady (coefficient of variation of inter-commit gaps)

```
blackbox rhythm              # last 30 days, pretty output
blackbox rhythm --days 7     # last 7 days
blackbox rhythm --format json
```

## License

MIT
