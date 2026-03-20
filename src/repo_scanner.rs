use anyhow::{Context, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &["node_modules", "target", ".build", "vendor"];

const WELL_KNOWN_DIRS: &[&str] = &[
    "Documents",
    "code",
    "projects",
    "src",
    "dev",
    "repos",
    "work",
    "github",
];

/// Check if a .git file is a valid worktree pointer (first line starts with 'gitdir:')
pub fn is_valid_gitdir_file(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| c.lines().next().map(|l| l.starts_with("gitdir:")))
        .unwrap_or(false)
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
    let first_line = content.lines().next().context("empty .git file")?;
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

pub fn discover_repos(watch_dirs: &[PathBuf], worktree_dir_name: Option<&str>) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    for dir in watch_dirs {
        // Fast path: dir is itself a repo root
        let git_path = dir.join(".git");
        if git_path.is_dir() {
            repos.push(dir.clone());
            // Scan only the worktree subdir (not the entire repo tree)
            if let Some(wt_name) = worktree_dir_name {
                let wt_dir = dir.join(wt_name);
                if wt_dir.is_dir() {
                    scan_repos_walkdir(&wt_dir, Some(2), &mut repos);
                }
            }
            continue;
        }
        if git_path.is_file() && is_valid_gitdir_file(&git_path) {
            repos.push(dir.clone());
            continue;
        }
        // Recursive WalkDir scan
        scan_repos_walkdir(dir, None, &mut repos);
    }
    repos.sort();
    repos.dedup();
    repos
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
    // Recursive WalkDir scan
    let mut repos = Vec::new();
    scan_repos_walkdir(dir, Some(max_depth), &mut repos);
    repos
}

fn scan_repos_walkdir(dir: &Path, max_depth: Option<usize>, repos: &mut Vec<PathBuf>) {
    let mut walker = WalkDir::new(dir).follow_links(false);
    if let Some(depth) = max_depth {
        walker = walker.max_depth(depth);
    }
    for entry in walker.into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !SKIP_DIRS.contains(&name.as_ref())
    }) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_name() == ".git" {
            let is_repo = entry.file_type().is_dir()
                || (entry.file_type().is_file() && is_valid_gitdir_file(entry.path()));
            if is_repo && let Some(parent) = entry.path().parent() {
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
