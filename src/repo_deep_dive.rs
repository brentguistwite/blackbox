use std::path::{Path, PathBuf};
use rusqlite::Connection;

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
