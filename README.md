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
| `today` | Show today's git activity (`--format pretty\|json\|csv`, `--summarize`) |
| `week` | Show this week's activity (`--format pretty\|json\|csv`, `--summarize`) |
| `month` | Show this month's activity (`--format pretty\|json\|csv`, `--summarize`) |
| `standup` | Slack/Teams-friendly activity summary (`--week`, `--summarize`) |
| `live` | Interactive TUI dashboard |
| `doctor` | Run health checks and report status |
| `install` | Register as OS service (launchd on macOS, systemd on Linux) |
| `uninstall` | Remove OS service registration |
| `hook <shell>` | Print shell hook script for zsh, bash, or fish |
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

**Pretty** (default):
```
Today - 3 repos, 12 commits, 4h 30m estimated

  myproject        8 commits   3h 15m
  other-repo       3 commits   1h 00m
  dotfiles         1 commit      15m
```

**JSON:** `blackbox today --format json` -- structured output with repo details, commit counts, time estimates, and PR info (when gh CLI available).

**CSV:** `blackbox today --format csv` -- flat rows suitable for spreadsheets/pipelines.

**Standup:** `blackbox standup` -- copy-paste-ready summary for Slack/Teams. Use `--week` for weekly and `--summarize` for LLM-generated summaries.

The `--summarize` flag is available on `today`, `week`, `month`, and `standup` to generate a natural-language summary of your activity using an LLM.

## How It Works

A background daemon polls your watched directories for git repos, recording commits, branch switches, and merges to a local SQLite database. The CLI queries this database and estimates time using a session-gap algorithm: commits within `session_gap_minutes` of each other belong to the same work session, and the first commit in each session gets a configurable time credit.

When shell hooks are installed (see above), blackbox also records directory presence — when you enter and leave a repo directory. Presence data anchors git session start times to when you actually started working, rather than relying on estimated credits. This produces more accurate time estimates, especially for repos where you spend significant time before your first commit.

When `gh` CLI is available, output is enriched with PR titles and URLs.

## License

MIT
