use assert_cmd::Command;

#[test]
fn test_cli_help() {
    Command::cargo_bin("blackbox")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
}

#[test]
fn test_init_placeholder() {
    // Wave 0 stub -- replaced by Plan 02
    panic!("Not yet implemented");
}
