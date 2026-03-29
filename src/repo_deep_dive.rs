use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use crate::query::{ActivityEvent, AiSessionInfo, ReviewInfo, TimeInterval, estimate_time_v2};
use chrono::Duration;

/// Canonicalize path, verify it's a git repo.
pub fn resolve_repo_path(input: &str) -> anyhow::Result<PathBuf> {
    let expanded = if input.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(input.replacen('~', &home, 1))
    } else {
        PathBuf::from(input)
    };

    let canonical = std::fs::canonicalize(&expanded)
        .map_err(|_| anyhow::anyhow!("Path not found: {input}"))?;

    git2::Repository::open(&canonical)
        .map_err(|_| anyhow::anyhow!("Not a git repository: {}", canonical.display()))?;

    Ok(canonical)
}

#[derive(Debug)]
pub struct RepoAllTimeData {
    pub repo_path: String,
    pub events: Vec<ActivityEvent>,
    pub reviews: Vec<ReviewInfo>,
    pub ai_sessions: Vec<AiSessionInfo>,
}

/// Query all git_activity, review_activity, ai_sessions for a repo (no time filter).
pub fn query_repo_all_time(conn: &Connection, repo_path: &str) -> anyhow::Result<RepoAllTimeData> {
    // git_activity
    let mut stmt = conn.prepare(
        "SELECT event_type, branch, commit_hash, message, timestamp
         FROM git_activity WHERE repo_path = ?1
         ORDER BY timestamp ASC",
    )?;
    let events: Vec<ActivityEvent> = stmt
        .query_map(rusqlite::params![repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(event_type, branch, commit_hash, message, ts_str)| {
            let timestamp = DateTime::parse_from_rfc3339(&ts_str).ok()?.with_timezone(&Utc);
            Some(ActivityEvent { event_type, branch, commit_hash, message, timestamp })
        })
        .collect();

    // review_activity
    let mut stmt = conn.prepare(
        "SELECT pr_number, pr_title, review_action, reviewed_at
         FROM review_activity WHERE repo_path = ?1
         ORDER BY reviewed_at ASC",
    )?;
    let reviews: Vec<ReviewInfo> = stmt
        .query_map(rusqlite::params![repo_path], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(pr_number, pr_title, action, reviewed_at_str)| {
            let reviewed_at = DateTime::parse_from_rfc3339(&reviewed_at_str).ok()?.with_timezone(&Utc);
            Some(ReviewInfo { pr_number, pr_title, action, reviewed_at })
        })
        .collect();

    // ai_sessions
    let now = Utc::now();
    let mut stmt = conn.prepare(
        "SELECT tool, session_id, started_at, ended_at, turns
         FROM ai_sessions WHERE repo_path = ?1
         ORDER BY started_at ASC",
    )?;
    let ai_sessions: Vec<AiSessionInfo> = stmt
        .query_map(rusqlite::params![repo_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(tool, session_id, started_str, ended_str, turns)| {
            let started_at = DateTime::parse_from_rfc3339(&started_str).ok()?.with_timezone(&Utc);
            let ended_at = ended_str.as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));
            let end = ended_at.unwrap_or(now);
            let duration = end - started_at;
            Some(AiSessionInfo { tool, session_id, started_at, ended_at, duration, turns })
        })
        .collect();

    Ok(RepoAllTimeData {
        repo_path: repo_path.to_string(),
        events,
        reviews,
        ai_sessions,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LanguageBreakdown {
    pub language: String,
    pub file_count: usize,
    pub line_count: usize,
    pub percent: f64,
}

/// Walk HEAD tree via git2, count lines per extension, map to languages.
pub fn compute_language_breakdown(repo_path: &Path) -> anyhow::Result<Vec<LanguageBreakdown>> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return Ok(vec![]),
    };
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return Ok(vec![]),
    };
    let tree = match head.peel_to_tree() {
        Ok(t) => t,
        Err(_) => return Ok(vec![]),
    };

    let mut ext_counts: HashMap<String, (usize, usize)> = HashMap::new();

    tree.walk(git2::TreeWalkMode::PreOrder, |_, entry| {
        if entry.kind() != Some(git2::ObjectType::Blob) {
            return git2::TreeWalkResult::Ok;
        }
        let name = match entry.name() {
            Some(n) => n,
            None => return git2::TreeWalkResult::Ok,
        };
        let ext = match Path::new(name).extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_lowercase(),
            None => return git2::TreeWalkResult::Ok,
        };
        let blob = match repo.find_blob(entry.id()) {
            Ok(b) => b,
            Err(_) => return git2::TreeWalkResult::Ok,
        };
        if blob.is_binary() {
            return git2::TreeWalkResult::Ok;
        }
        let lines = blob.content().split(|&b| b == b'\n').count();
        let e = ext_counts.entry(ext).or_insert((0, 0));
        e.0 += 1;
        e.1 += lines;
        git2::TreeWalkResult::Ok
    })?;

    let total_lines: usize = ext_counts.values().map(|(_, l)| l).sum();
    if total_lines == 0 {
        return Ok(vec![]);
    }

    let mut lang_map: HashMap<String, (usize, usize)> = HashMap::new();
    for (ext, (fc, lc)) in &ext_counts {
        let lang = ext_to_language(ext);
        let e = lang_map.entry(lang).or_insert((0, 0));
        e.0 += fc;
        e.1 += lc;
    }

    let mut result: Vec<LanguageBreakdown> = lang_map
        .into_iter()
        .map(|(language, (file_count, line_count))| LanguageBreakdown {
            language,
            file_count,
            line_count,
            percent: line_count as f64 / total_lines as f64 * 100.0,
        })
        .collect();
    result.sort_by(|a, b| b.line_count.cmp(&a.line_count));
    Ok(result)
}

fn ext_to_language(ext: &str) -> String {
    match ext {
        "rs" => "Rust",
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" => "JavaScript",
        "py" => "Python",
        "go" => "Go",
        "java" => "Java",
        "kt" => "Kotlin",
        "swift" => "Swift",
        "rb" => "Ruby",
        "c" => "C",
        "cpp" | "cc" => "C++",
        "h" => "C/C++ Header",
        "cs" => "C#",
        "sh" | "bash" | "zsh" | "fish" => "Shell",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "json" => "JSON",
        "md" => "Markdown",
        "html" => "HTML",
        "css" | "scss" => "CSS",
        "sql" => "SQL",
        other => other,
    }
    .to_string()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FileChurnEntry {
    pub path: String,
    pub change_count: usize,
}

/// Walk commits from HEAD, count file appearances in diffs. Cap at 500 commits.
pub fn compute_top_files(repo_path: &Path, limit: usize) -> anyhow::Result<Vec<FileChurnEntry>> {
    let repo = git2::Repository::open(repo_path)?;
    let mut revwalk = repo.revwalk()?;
    if revwalk.push_head().is_err() {
        // empty repo — no HEAD
        return Ok(vec![]);
    }
    revwalk.set_sorting(git2::Sort::TIME)?;

    let mut file_counts: HashMap<String, usize> = HashMap::new();

    for (walked, oid_result) in revwalk.enumerate() {
        if walked >= 500 {
            log::debug!("compute_top_files: capped at 500 commits");
            break;
        }
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;

        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            commit.parent(0).ok().and_then(|p| p.tree().ok())
        } else {
            None
        };

        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        diff.foreach(
            &mut |delta, _| {
                if delta.new_file().is_binary() {
                    return true;
                }
                if let Some(p) = delta.new_file().path().and_then(|p| p.to_str()) {
                    *file_counts.entry(p.to_string()).or_insert(0) += 1;
                }
                true
            },
            None,
            None,
            None,
        )?;
    }

    let mut entries: Vec<FileChurnEntry> = file_counts
        .into_iter()
        .map(|(path, change_count)| FileChurnEntry { path, change_count })
        .collect();
    entries.sort_by(|a, b| b.change_count.cmp(&a.change_count));
    entries.truncate(limit);
    Ok(entries)
}

/// Compute total estimated time invested using estimate_time_v2.
/// No time-window clipping — uses full AI session durations.
pub fn compute_time_invested(
    data: &RepoAllTimeData,
    session_gap_minutes: u64,
    first_commit_minutes: u64,
) -> Duration {
    if data.events.is_empty() && data.ai_sessions.is_empty() {
        return Duration::zero();
    }

    let now = chrono::Utc::now();
    let ai_intervals: Vec<TimeInterval> = data
        .ai_sessions
        .iter()
        .filter_map(|s| {
            let end = s.ended_at.unwrap_or(now);
            if s.started_at < end {
                Some(TimeInterval { start: s.started_at, end })
            } else {
                None
            }
        })
        .collect();

    let (duration, _) = estimate_time_v2(&data.events, &ai_intervals, &[], session_gap_minutes, first_commit_minutes);
    duration
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BranchActivity {
    pub name: String,
    pub commit_count: usize,
    pub last_active: DateTime<Utc>,
}

/// Summarize branch activity from DB events: commit count + last active per branch.
/// Includes branches from branch_switch events (with commit_count=0 if no commits).
pub fn compute_branch_activity(data: &RepoAllTimeData) -> Vec<BranchActivity> {
    // track (commit_count, max_timestamp) per branch
    let mut branches: HashMap<String, (usize, DateTime<Utc>)> = HashMap::new();

    for event in &data.events {
        let name = match &event.branch {
            Some(n) => n,
            None => continue,
        };

        let entry = branches.entry(name.clone()).or_insert((0, event.timestamp));

        if event.event_type == "commit" {
            entry.0 += 1;
        }

        if event.timestamp > entry.1 {
            entry.1 = event.timestamp;
        }
    }

    let mut result: Vec<BranchActivity> = branches
        .into_iter()
        .map(|(name, (commit_count, last_active))| BranchActivity {
            name,
            commit_count,
            last_active,
        })
        .collect();
    result.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    result
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RepoPrEntry {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub branch: Option<String>,
    pub url: Option<String>,
}

/// Check once if `gh` CLI is available on PATH.
fn gh_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("which")
            .arg("gh")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

/// Fetch all PRs for a repo via `gh pr list --state all`. Returns None on any failure.
fn fetch_all_prs_gh(repo_path: &str) -> Option<Vec<serde_json::Value>> {
    let child = Command::new("gh")
        .args([
            "pr", "list", "--state", "all",
            "--json", "number,title,state,headRefName,url",
            "--limit", "30",
        ])
        .current_dir(repo_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let handle = std::thread::spawn(move || child.wait_with_output());
    match handle.join() {
        Ok(Ok(output)) if output.status.success() => {
            serde_json::from_slice(&output.stdout).ok()
        }
        _ => None,
    }
}

/// Fetch PR history combining DB reviews + live gh CLI data.
/// Graceful degradation: returns review-only list if gh unavailable/fails.
pub fn fetch_repo_pr_history(repo_path: &str, data: &RepoAllTimeData) -> Vec<RepoPrEntry> {
    // Step 1: collect unique PRs from DB reviews
    let mut pr_map: HashMap<u64, RepoPrEntry> = HashMap::new();
    for review in &data.reviews {
        pr_map.entry(review.pr_number as u64).or_insert_with(|| RepoPrEntry {
            number: review.pr_number as u64,
            title: review.pr_title.clone(),
            state: "REVIEWED".to_string(),
            branch: None,
            url: None,
        });
    }

    // Step 2: try gh CLI for live data
    if gh_available()
        && let Some(gh_prs) = fetch_all_prs_gh(repo_path)
    {
        for val in gh_prs {
            let number = match val.get("number").and_then(|v| v.as_u64()) {
                Some(n) => n,
                None => continue,
            };
            let title = val.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let state = val.get("state").and_then(|v| v.as_str()).unwrap_or("OPEN").to_string();
            let branch = val.get("headRefName").and_then(|v| v.as_str()).map(String::from);
            let url = val.get("url").and_then(|v| v.as_str()).map(String::from);

            // Step 3: upsert — gh data takes precedence
            pr_map.insert(number, RepoPrEntry { number, title, state, branch, url });
        }
    }

    // Sort by number descending
    let mut result: Vec<RepoPrEntry> = pr_map.into_values().collect();
    result.sort_by(|a, b| b.number.cmp(&a.number));
    result
}

#[derive(Debug)]
pub struct RepoDeepDive {
    pub repo_path: String,
    pub repo_name: String,
    pub total_commits: usize,
    pub first_commit_at: Option<DateTime<Utc>>,
    pub last_commit_at: Option<DateTime<Utc>>,
    pub total_estimated_time: Duration,
    pub languages: Vec<LanguageBreakdown>,
    pub top_files: Vec<FileChurnEntry>,
    pub branches: Vec<BranchActivity>,
    pub prs: Vec<RepoPrEntry>,
    pub tracked: bool,
}

/// Orchestrate all sub-computations into a single RepoDeepDive.
pub fn build_deep_dive(
    repo_path_input: &str,
    conn: &Connection,
    config: &crate::config::Config,
) -> anyhow::Result<RepoDeepDive> {
    let canonical = resolve_repo_path(repo_path_input)?;
    let db_repo_path = find_db_repo_path(conn, &canonical)?;
    let tracked = db_repo_path.is_some();
    let effective_path_owned =
        db_repo_path.unwrap_or_else(|| canonical.to_string_lossy().to_string());

    let data = query_repo_all_time(conn, &effective_path_owned)?;

    let languages = compute_language_breakdown(&canonical).unwrap_or_default();
    let top_files = compute_top_files(&canonical, 10).unwrap_or_default();
    let total_estimated_time =
        compute_time_invested(&data, config.session_gap_minutes, config.first_commit_minutes);
    let branches = compute_branch_activity(&data);
    let prs = fetch_repo_pr_history(&effective_path_owned, &data);

    let commit_times: Vec<_> = data
        .events
        .iter()
        .filter(|e| e.event_type == "commit")
        .map(|e| e.timestamp)
        .collect();
    let first_commit_at = commit_times.iter().copied().min();
    let last_commit_at = commit_times.iter().copied().max();
    let total_commits = commit_times.len();

    let repo_name = std::path::Path::new(&effective_path_owned)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| effective_path_owned.clone());

    Ok(RepoDeepDive {
        repo_path: effective_path_owned,
        repo_name,
        total_commits,
        first_commit_at,
        last_commit_at,
        total_estimated_time,
        languages,
        top_files,
        branches,
        prs,
        tracked,
    })
}

/// Look up repo_path in DB via exact or prefix match.
pub fn find_db_repo_path(conn: &Connection, canonical: &Path) -> anyhow::Result<Option<String>> {
    let path_str = canonical.to_string_lossy();
    let result: rusqlite::Result<String> = conn.query_row(
        "SELECT DISTINCT repo_path FROM git_activity WHERE repo_path = ?1 OR repo_path LIKE ?1 || '/%' LIMIT 1",
        rusqlite::params![path_str],
        |row| row.get(0),
    );
    match result {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
