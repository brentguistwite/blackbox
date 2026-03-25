use blackbox::config::Config;
use blackbox::llm;

#[test]
fn test_build_llm_config_no_key_errors() {
    let config = Config::default();
    let result = llm::build_llm_config(&config);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("No LLM API key configured"), "got: {}", err);
    assert!(
        err.contains("llm_api_key"),
        "should show config hint, got: {}",
        err
    );
}

#[test]
fn test_build_llm_config_anthropic_defaults() {
    let config = Config {
        llm_api_key: Some("sk-test-key".to_string()),
        ..Config::default()
    };
    let llm_config = llm::build_llm_config(&config).unwrap();
    assert_eq!(llm_config.provider, "anthropic");
    assert_eq!(llm_config.api_key, "sk-test-key");
    assert!(
        llm_config.model.contains("claude"),
        "default model should be claude, got: {}",
        llm_config.model
    );
    assert!(llm_config.base_url.is_none());
}

#[test]
fn test_build_llm_config_openai_defaults() {
    let config = Config {
        llm_api_key: Some("sk-openai-key".to_string()),
        llm_provider: Some("openai".to_string()),
        ..Config::default()
    };
    let llm_config = llm::build_llm_config(&config).unwrap();
    assert_eq!(llm_config.provider, "openai");
    assert_eq!(llm_config.model, "gpt-4o-mini");
}

#[test]
fn test_build_llm_config_custom_model() {
    let config = Config {
        llm_api_key: Some("key".to_string()),
        llm_model: Some("claude-opus-4-20250514".to_string()),
        ..Config::default()
    };
    let llm_config = llm::build_llm_config(&config).unwrap();
    assert_eq!(llm_config.model, "claude-opus-4-20250514");
}

#[test]
fn test_build_llm_config_custom_base_url() {
    let config = Config {
        llm_api_key: Some("key".to_string()),
        llm_provider: Some("openai".to_string()),
        llm_base_url: Some("http://localhost:8080".to_string()),
        ..Config::default()
    };
    let llm_config = llm::build_llm_config(&config).unwrap();
    assert_eq!(
        llm_config.base_url.as_deref(),
        Some("http://localhost:8080")
    );
}

#[test]
fn test_system_prompt_is_concise() {
    assert!(llm::SYSTEM_PROMPT.contains("summarizing"));
    assert!(llm::SYSTEM_PROMPT.contains("3-5 sentences"));
}

#[test]
fn test_summarize_unsupported_provider() {
    let llm_config = llm::LlmConfig {
        provider: "gemini".to_string(),
        api_key: "key".to_string(),
        model: "model".to_string(),
        base_url: None,
    };
    let result = llm::summarize_activity(&llm_config, "{}");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Unsupported LLM provider"), "got: {}", err);
    assert!(err.contains("gemini"), "got: {}", err);
}
