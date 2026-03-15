use std::path::PathBuf;
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &["node_modules", "target", ".build", "vendor"];

pub fn discover_repos(watch_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    for dir in watch_dirs {
        for entry in WalkDir::new(dir)
            .follow_links(false)
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
            if entry.file_type().is_dir() && entry.file_name() == ".git" {
                if let Some(parent) = entry.path().parent() {
                    // Skip bare repos
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
    repos
}
