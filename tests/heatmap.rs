use assert_cmd::Command;
use tempfile::TempDir;

#[test]
fn heatmap_exits_zero_with_empty_db() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(config_dir.join("blackbox")).unwrap();
    std::fs::write(
        config_dir.join("blackbox").join("config.toml"),
        "watch_dirs = []\npoll_interval_secs = 60\n",
    )
    .unwrap();

    // open_db creates the DB + runs migrations
    let db_path = data_dir.join("blackbox").join("blackbox.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let _conn = blackbox::db::open_db(&db_path).unwrap();

    Command::cargo_bin("blackbox")
        .unwrap()
        .arg("heatmap")
        .env("XDG_DATA_HOME", &data_dir)
        .env("XDG_CONFIG_HOME", &config_dir)
        .assert()
        .success();
}
