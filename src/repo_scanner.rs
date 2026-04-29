use anyhow::{bail, Context};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &["node_modules", "target", ".build", "vendor"];

const WELL_KNOWN_DIRS: &[&str] = &[
    "Documents", "code", "projects", "src", "dev", "repos", "work", "github",
];

/// Result of probing a `.git` pointer file. Distinguishing unreadable from
/// invalid lets discovery surface a transient TCC denial as a traversal
/// error (Required) instead of silently dropping the worktree from `repos`
/// — Codex round 10 [high]. Without this, a permission flap would prune
/// repo_states, and the next clean poll re-enters first-poll mode (HEAD +
/// today's first 50 commits only), losing every commit during the outage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitdirFileProbe {
    Valid,
    Invalid,
    Unreadable,
}

/// Probe a `.git` pointer file. Distinguishes successful read with valid /
/// invalid contents from an IO/permission failure.
pub fn probe_gitdir_file(path: &Path) -> GitdirFileProbe {
    match std::fs::read_to_string(path) {
        Ok(c) => {
            if c.lines().next().is_some_and(|l| l.starts_with("gitdir:")) {
                GitdirFileProbe::Valid
            } else {
                GitdirFileProbe::Invalid
            }
        }
        Err(_) => GitdirFileProbe::Unreadable,
    }
}

/// Check if a .git file is a valid worktree pointer (first line starts with
/// 'gitdir:'). Returns false for both invalid contents AND unreadable files;
/// callers that need to distinguish those cases should use
/// `probe_gitdir_file` directly.
pub fn is_valid_gitdir_file(path: &Path) -> bool {
    matches!(probe_gitdir_file(path), GitdirFileProbe::Valid)
}

/// Check if path is a git worktree (has .git file with gitdir: pointer).
/// Returns Some(resolved_gitdir_path) for worktrees (e.g. /main/.git/worktrees/<name>),
/// None for regular repos or non-repos.
pub fn is_worktree(path: &Path) -> Option<PathBuf> {
    let git_path = path.join(".git");
    if !git_path.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(&git_path).ok()?;
    let first_line = content.lines().next()?;
    let raw = first_line.strip_prefix("gitdir: ")?;
    let gitdir = if Path::new(raw).is_relative() {
        path.join(raw)
    } else {
        PathBuf::from(raw)
    };
    let resolved = gitdir.canonicalize().ok()?;
    Some(resolved)
}

/// Resolve a worktree path to its main repo root.
/// Reads .git file → parses gitdir → 3x .parent() (removes <name>, worktrees, .git) → validates.
pub fn resolve_main_repo(worktree_path: &Path) -> anyhow::Result<PathBuf> {
    let git_path = worktree_path.join(".git");
    let content = std::fs::read_to_string(&git_path)
        .with_context(|| format!("failed to read {}", git_path.display()))?;
    let first_line = content
        .lines()
        .next()
        .context("empty .git file")?;
    let raw = first_line
        .strip_prefix("gitdir: ")
        .context("missing 'gitdir:' prefix in .git file")?;
    let gitdir = if Path::new(raw).is_relative() {
        worktree_path.join(raw)
    } else {
        PathBuf::from(raw)
    };
    let resolved = gitdir
        .canonicalize()
        .with_context(|| format!("gitdir path does not exist: {}", gitdir.display()))?;
    // 3x .parent(): <name> → worktrees → .git → repo root
    let main_root = resolved
        .parent() // remove <worktree-name>
        .and_then(|p| p.parent()) // remove "worktrees"
        .and_then(|p| p.parent()) // remove ".git"
        .context("failed to resolve main repo root from gitdir path")?;
    // Validate
    if !main_root.join(".git").is_dir() {
        bail!(
            "resolved path {} does not contain a .git directory",
            main_root.display()
        );
    }
    Ok(main_root.to_path_buf())
}

/// Find worktree parent directories (e.g. `.worktrees/`) for main repos.
/// Skips worktrees themselves — only returns dirs for non-worktree repos.
pub fn find_worktree_parent_dirs(repos: &[PathBuf], worktree_dir_name: &str) -> Vec<PathBuf> {
    repos
        .iter()
        .filter(|r| is_worktree(r).is_none()) // only main repos
        .map(|r| r.join(worktree_dir_name))
        .filter(|p| p.is_dir())
        .collect()
}

/// Outcome of a discovery pass: discovered repos plus any subtree errors.
/// Traversal errors mean parts of the watch tree couldn't be enumerated —
/// repos under those subtrees are absent from `repos` and tracking will
/// silently miss them unless the daemon surfaces this. doctor reads
/// these via DiscoveryMetrics.
#[derive(Debug, Default, Clone)]
pub struct DiscoveredRepos {
    pub repos: Vec<PathBuf>,
    /// Errors that actually block discovery of additional repo roots — the
    /// watch_dir tree itself, paths NOT under any discovered repo, or paths
    /// under a worktree-parent dir (worktrees ARE discoverable repos).
    /// doctor escalates these to Required.
    pub traversal_errors: Vec<(PathBuf, String)>,
    /// Errors deep inside a discovered repo's tree (artifact dirs, generated
    /// caches, restricted private subtrees). These are NOT discovery failures
    /// — the repo polled fine — but kept for diagnostics. doctor / status do
    /// not promote them to Required. Codex round 9 [medium]: a chmod-000
    /// artifact dir inside a healthy repo was flipping daemon Required/Red.
    pub inside_repo_advisories: Vec<(PathBuf, String)>,
}

pub fn discover_repos(watch_dirs: &[PathBuf], worktree_dir_name: Option<&str>) -> Vec<PathBuf> {
    discover_repos_with_errors(watch_dirs, worktree_dir_name).repos
}

/// Same as `discover_repos` but also returns paths that errored during
/// recursive walk (e.g. TCC-denied subdirectories). Use this from the
/// daemon's full_scan so subtree denial doesn't silently shrink the
/// discovered set with no health signal.
pub fn discover_repos_with_errors(
    watch_dirs: &[PathBuf],
    worktree_dir_name: Option<&str>,
) -> DiscoveredRepos {
    let mut repos = Vec::new();
    let mut errors: Vec<(PathBuf, String)> = Vec::new();
    let mut worktree_parents: Vec<PathBuf> = Vec::new();
    for dir in watch_dirs {
        // Fast path: dir is itself a repo root
        let git_path = dir.join(".git");
        if git_path.is_dir() {
            repos.push(dir.clone());
            // Scan only the worktree subdir (not the entire repo tree)
            if let Some(wt_name) = worktree_dir_name {
                let wt_dir = dir.join(wt_name);
                if wt_dir.is_dir() {
                    worktree_parents.push(wt_dir.clone());
                    scan_repos_walkdir(&wt_dir, Some(2), &mut repos, &mut errors);
                }
            }
            continue;
        }
        if git_path.is_file() {
            match probe_gitdir_file(&git_path) {
                GitdirFileProbe::Valid => {
                    repos.push(dir.clone());
                    continue;
                }
                GitdirFileProbe::Unreadable => {
                    // Codex round 10 [high]: a TCC denial / chmod 000 on a
                    // worktree's .git pointer must NOT silently drop the
                    // worktree. Surface as discovery failure so doctor /
                    // status flag Required and prune treats `dir` as an
                    // unreliable_root that keeps RepoState.
                    errors.push((git_path.clone(), "unreadable .git pointer".into()));
                    continue;
                }
                GitdirFileProbe::Invalid => {
                    // Genuinely not a repo (some other .git file).
                }
            }
        }
        // Recursive WalkDir scan
        scan_repos_walkdir(dir, None, &mut repos, &mut errors);
    }
    repos.sort();
    repos.dedup();
    let _ = worktree_parents;

    // Codex round 10 [medium]: do NOT downgrade walk errors to advisory just
    // because they fall under a discovered repo's tree. scan_repos_walkdir
    // descends recursively and discovers nested repos under any subtree;
    // suppressing those errors lets a permission-denied subtree hide a
    // nested repo with no Required signal. False positive is the right
    // failure mode here — silent loss is not. Round 9's split was wrong;
    // round 8's "surface everything" was correct. (Codex 6/7/8/9/10 have
    // oscillated on this; we're locking in round 10's recommendation.)
    DiscoveredRepos {
        repos,
        traversal_errors: errors,
        inside_repo_advisories: Vec::new(),
    }
}

/// Scan well-known dev directories + HOME children for git repos.
/// Returns (parent_dir, repos) tuples grouped by scanned directory.
pub fn auto_scan_repos() -> Vec<(PathBuf, Vec<PathBuf>)> {
    let home = match etcetera::home_dir() {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    auto_scan_repos_from(&home)
}

/// Testable version that accepts a home path.
pub fn auto_scan_repos_from(home: &Path) -> Vec<(PathBuf, Vec<PathBuf>)> {
    if !home.is_dir() {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut scanned = HashSet::new();

    // Scan well-known directories (deeper: 4 levels)
    for dir_name in WELL_KNOWN_DIRS {
        let dir = home.join(dir_name);
        if dir.is_dir() {
            let repos = discover_repos_limited(&dir, 4);
            if !repos.is_empty() {
                results.push((dir.clone(), repos));
            }
            scanned.insert(dir);
        }
    }

    // Scan direct children of HOME (shallower: 3 levels)
    let entries = match std::fs::read_dir(home) {
        Ok(e) => e,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || scanned.contains(&path) {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden dirs except .config, .local
        if name.starts_with('.') && name != ".config" && name != ".local" {
            continue;
        }
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }

        let repos = discover_repos_limited(&path, 3);
        if !repos.is_empty() {
            results.push((path, repos));
        }
    }

    results
}

/// Scan a single directory for git repos (depth limit 4).
/// Used by setup wizard to scan user-specified directories.
pub fn scan_directory(dir: &Path) -> Vec<PathBuf> {
    discover_repos_limited(dir, 4)
}

fn discover_repos_limited(dir: &Path, max_depth: usize) -> Vec<PathBuf> {
    // Fast path: dir is itself a repo root
    let git_path = dir.join(".git");
    if git_path.is_dir() {
        return vec![dir.to_path_buf()];
    }
    if git_path.is_file() && is_valid_gitdir_file(&git_path) {
        return vec![dir.to_path_buf()];
    }
    // Recursive WalkDir scan — errors discarded for non-daemon callers
    // (auto_scan_repos, scan_directory) where surfacing them would change
    // wizard UX. Daemon path uses discover_repos_with_errors instead.
    let mut repos = Vec::new();
    let mut _errors = Vec::new();
    scan_repos_walkdir(dir, Some(max_depth), &mut repos, &mut _errors);
    repos
}

fn scan_repos_walkdir(
    dir: &Path,
    max_depth: Option<usize>,
    repos: &mut Vec<PathBuf>,
    errors: &mut Vec<(PathBuf, String)>,
) {
    let mut walker = WalkDir::new(dir).follow_links(false);
    if let Some(depth) = max_depth {
        walker = walker.max_depth(depth);
    }
    for entry in walker
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !SKIP_DIRS.contains(&name.as_ref())
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(walk_err) => {
                // walkdir's Error carries the path that errored (when
                // available) and the underlying io::Error. Capture both
                // so doctor can surface subtree TCC denial / permission
                // changes that previously vanished into thin air.
                let path = walk_err.path().map(|p| p.to_path_buf()).unwrap_or_else(|| dir.to_path_buf());
                let msg = walk_err
                    .io_error()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| walk_err.to_string());
                errors.push((path, msg));
                continue;
            }
        };
        if entry.file_name() == ".git" {
            let is_repo = if entry.file_type().is_dir() {
                true
            } else if entry.file_type().is_file() {
                match probe_gitdir_file(entry.path()) {
                    GitdirFileProbe::Valid => true,
                    GitdirFileProbe::Unreadable => {
                        // Codex round 10 [high]: surface unreadable .git
                        // pointer as discovery error so doctor / status flag
                        // Required and prune skips state eviction.
                        errors.push((
                            entry.path().to_path_buf(),
                            "unreadable .git pointer".into(),
                        ));
                        false
                    }
                    GitdirFileProbe::Invalid => false,
                }
            } else {
                false
            };
            if is_repo
                && let Some(parent) = entry.path().parent()
            {
                match git2::Repository::open(parent) {
                    Ok(repo) if !repo.is_bare() => {
                        repos.push(parent.to_path_buf());
                    }
                    _ => {}
                }
            }
        }
    }
}
