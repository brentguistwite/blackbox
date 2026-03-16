use blackbox::config::{self, Config};
use std::path::PathBuf;

#[test]
fn test_default_config() {
    let cfg = Config::default();
    assert!(cfg.watch_dirs.is_empty());
    assert_eq!(cfg.poll_interval_secs, 300);
}

#[test]
fn test_parse_valid_toml() {
    let toml_str = r#"
        watch_dirs = ["/home/user/code", "/home/user/projects"]
        poll_interval_secs = 60
    "#;
    let cfg: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.watch_dirs.len(), 2);
    assert_eq!(cfg.watch_dirs[0], PathBuf::from("/home/user/code"));
    assert_eq!(cfg.poll_interval_secs, 60);
}

#[test]
fn test_parse_missing_fields_uses_defaults() {
    let toml_str = r#"
        watch_dirs = ["/tmp/code"]
    "#;
    let cfg: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.poll_interval_secs, 300);
    assert_eq!(cfg.watch_dirs.len(), 1);
}

#[test]
fn test_validate_poll_interval_too_low() {
    let cfg = Config {
        watch_dirs: vec![],
        poll_interval_secs: 5,
        ..Config::default()
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn test_validate_empty_config_ok() {
    let cfg = Config::default();
    assert!(cfg.validate().is_ok());
}

#[test]
fn test_config_dir_returns_blackbox_subdir() {
    let dir = config::config_dir().unwrap();
    assert!(dir.ends_with("blackbox"));
}

#[test]
fn test_data_dir_returns_blackbox_subdir() {
    let dir = config::data_dir().unwrap();
    assert!(dir.ends_with("blackbox"));
}

#[test]
fn test_tilde_expansion() {
    let mut cfg = Config {
        watch_dirs: vec![PathBuf::from("~/code")],
        poll_interval_secs: 300,
        ..Config::default()
    };
    cfg.expand_paths();
    let expanded = &cfg.watch_dirs[0];
    assert!(!expanded.starts_with("~"), "Path should not start with ~");
    assert!(expanded.is_absolute(), "Path should be absolute after expansion");
}

#[test]
fn test_parse_llm_fields() {
    let toml_str = r#"
        watch_dirs = ["/tmp/code"]
        llm_provider = "anthropic"
        llm_api_key = "sk-test-123"
        llm_model = "claude-sonnet-4-20250514"
    "#;
    let cfg: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(cfg.llm_api_key.as_deref(), Some("sk-test-123"));
    assert_eq!(cfg.llm_model.as_deref(), Some("claude-sonnet-4-20250514"));
    assert!(cfg.llm_base_url.is_none());
}

#[test]
fn test_parse_llm_fields_defaults_to_none() {
    let toml_str = r#"
        watch_dirs = ["/tmp/code"]
    "#;
    let cfg: Config = toml::from_str(toml_str).unwrap();
    assert!(cfg.llm_provider.is_none());
    assert!(cfg.llm_api_key.is_none());
    assert!(cfg.llm_model.is_none());
    assert!(cfg.llm_base_url.is_none());
}

#[test]
fn test_parse_llm_base_url() {
    let toml_str = r#"
        watch_dirs = []
        llm_provider = "openai"
        llm_api_key = "key"
        llm_base_url = "http://localhost:11434"
    "#;
    let cfg: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.llm_base_url.as_deref(), Some("http://localhost:11434"));
}
