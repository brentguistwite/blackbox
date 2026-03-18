use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &["node_modules", "target", ".build", "vendor"];

const WELL_KNOWN_DIRS: &[&str] = &[
    "Documents", "code", "projects", "src", "dev", "repos", "work", "github",
];

/// Check if a .git file is a valid worktree pointer (first line starts with 'gitdir:')
pub fn is_valid_gitdir_file(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| c.lines().next().map(|l| l.starts_with("gitdir:")))
        .unwrap_or(false)
}

pub fn discover_repos(watch_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    for dir in watch_dirs {
        // Fast path: dir is itself a repo root
        let git_path = dir.join(".git");
        if git_path.is_dir() {
            repos.push(dir.clone());
            continue;
        }
        if git_path.is_file() && is_valid_gitdir_file(&git_path) {
            repos.push(dir.clone());
            continue;
        }
        // Recursive WalkDir scan
        scan_repos_walkdir(dir, None, &mut repos);
    }
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
    for entry in walker
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !SKIP_DIRS.contains(&name.as_ref())
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_name() == ".git" {
            let is_repo = entry.file_type().is_dir()
                || (entry.file_type().is_file() && is_valid_gitdir_file(entry.path()));
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
