use blackbox::db::open_db;
use tempfile::NamedTempFile;

fn setup_db() -> (rusqlite::Connection, NamedTempFile) {
    let f = NamedTempFile::new().unwrap();
    let conn = open_db(f.path()).unwrap();
    (conn, f)
}

#[test]
fn pr_snapshots_table_exists_and_accepts_inserts() {
    let (conn, _tmp) = setup_db();

    conn.execute(
        "INSERT INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref,
          author_login, created_at_gh, merged_at, closed_at, first_review_at,
          additions, deletions, commits, iteration_count)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        rusqlite::params![
            "/repo/test",
            1_i64,
            "Test PR",
            "https://github.com/test/repo/pull/1",
            "MERGED",
            "feature/test",
            "main",
            "testuser",
            "2025-01-15T10:00:00Z",
            "2025-01-15T12:00:00Z",
            rusqlite::types::Null,
            "2025-01-15T10:30:00Z",
            100_i64,
            20_i64,
            5_i64,
            1_i64,
        ],
    )
    .unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn pr_snapshots_unique_index_replaces_on_conflict() {
    let (conn, _tmp) = setup_db();

    // Insert initial row
    conn.execute(
        "INSERT INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params!["/repo/test", 1_i64, "Original", "https://url", "OPEN", "feat", "main"],
    )
    .unwrap();

    // INSERT OR REPLACE with same (repo_path, pr_number) — should replace
    conn.execute(
        "INSERT OR REPLACE INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params!["/repo/test", 1_i64, "Updated", "https://url", "MERGED", "feat", "main"],
    )
    .unwrap();

    let (title, state): (String, String) = conn
        .query_row(
            "SELECT title, state FROM pr_snapshots WHERE repo_path = ?1 AND pr_number = ?2",
            rusqlite::params!["/repo/test", 1_i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(title, "Updated");
    assert_eq!(state, "MERGED");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "should have 1 row after replace, not 2");
}

#[test]
fn pr_snapshots_nullable_fields_accept_null() {
    let (conn, _tmp) = setup_db();

    // All optional fields as NULL
    conn.execute(
        "INSERT INTO pr_snapshots
         (repo_path, pr_number, title, url, state, head_ref, base_ref,
          author_login, created_at_gh, merged_at, closed_at, first_review_at,
          additions, deletions, commits, iteration_count)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        rusqlite::params![
            "/repo/test",
            1_i64,
            "PR with nulls",
            "https://url",
            "OPEN",
            "feat",
            "main",
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
            rusqlite::types::Null,
        ],
    )
    .unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pr_snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
