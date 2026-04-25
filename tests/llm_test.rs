use blackbox::config::Config;
use blackbox::llm;

#[test]
fn test_build_llm_config_no_key_errors() {
    let config = Config::default();
    // Use DI variant with empty env reader so host-set ANTHROPIC_API_KEY
    // doesn't flake this test.
    let result = llm::build_llm_config_with_env(&config, |_| None);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("No LLM API key configured"), "got: {}", err);
    assert!(err.contains("llm_api_key"), "should show config hint, got: {}", err);
}

#[test]
fn test_build_llm_config_env_var_anthropic_fallback() {
    let config = Config::default();
    let result = llm::build_llm_config_with_env(&config, |k| {
        (k == "ANTHROPIC_API_KEY").then(|| "sk-ant-env-key-xyz".to_string())
    });
    let cfg = result.expect("env var should satisfy key requirement");
    assert_eq!(cfg.provider, "anthropic");
    assert_eq!(cfg.api_key, "sk-ant-env-key-xyz");
}

#[test]
fn test_build_llm_config_env_var_openai_fallback() {
    let config = Config {
        llm_provider: Some("openai".to_string()),
        ..Config::default()
    };
    let result = llm::build_llm_config_with_env(&config, |k| {
        (k == "OPENAI_API_KEY").then(|| "sk-openai-env-key".to_string())
    });
    let cfg = result.expect("OPENAI_API_KEY should satisfy openai provider");
    assert_eq!(cfg.api_key, "sk-openai-env-key");
}

#[test]
fn test_build_llm_config_config_key_wins_over_env() {
    let config = Config {
        llm_api_key: Some("from-config".to_string()),
        ..Config::default()
    };
    let cfg = llm::build_llm_config_with_env(&config, |_| Some("from-env".to_string())).unwrap();
    assert_eq!(cfg.api_key, "from-config", "config should take precedence over env");
}

#[test]
fn test_build_llm_config_auto_detects_openai_from_env_when_provider_unset() {
    // Codex finding: OPENAI_API_KEY alone, no llm_provider, must work end-to-end.
    let config = Config::default(); // provider None, no api_key
    let cfg = llm::build_llm_config_with_env(&config, |k| match k {
        "OPENAI_API_KEY" => Some("sk-openai-only".to_string()),
        _ => None,
    })
    .expect("OPENAI_API_KEY alone should be enough to configure LLM");
    assert_eq!(cfg.provider, "openai", "should auto-detect openai provider");
    assert_eq!(cfg.api_key, "sk-openai-only");
}

#[test]
fn test_build_llm_config_auto_detects_anthropic_from_env_when_provider_unset() {
    let config = Config::default();
    let cfg = llm::build_llm_config_with_env(&config, |k| match k {
        "ANTHROPIC_API_KEY" => Some("sk-ant-only".to_string()),
        _ => None,
    })
    .expect("ANTHROPIC_API_KEY alone should configure LLM");
    assert_eq!(cfg.provider, "anthropic");
    assert_eq!(cfg.api_key, "sk-ant-only");
}

#[test]
fn test_build_llm_config_auto_detect_both_set_prefers_anthropic() {
    // Tie-breaker: anthropic is the default in existing config semantics.
    let config = Config::default();
    let cfg = llm::build_llm_config_with_env(&config, |k| match k {
        "ANTHROPIC_API_KEY" => Some("sk-ant-both".to_string()),
        "OPENAI_API_KEY" => Some("sk-openai-both".to_string()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.provider, "anthropic", "when both env vars set, prefer anthropic default");
    assert_eq!(cfg.api_key, "sk-ant-both");
}

#[test]
fn test_build_llm_config_explicit_provider_overrides_auto_detect() {
    // If user explicitly set llm_provider = "openai", don't grab ANTHROPIC_API_KEY.
    let config = Config {
        llm_provider: Some("openai".to_string()),
        ..Config::default()
    };
    let cfg = llm::build_llm_config_with_env(&config, |k| match k {
        "ANTHROPIC_API_KEY" => Some("sk-ant-wrong".to_string()),
        "OPENAI_API_KEY" => Some("sk-openai-right".to_string()),
        _ => None,
    })
    .unwrap();
    assert_eq!(cfg.provider, "openai");
    assert_eq!(cfg.api_key, "sk-openai-right");
}

#[test]
fn test_build_llm_config_error_mentions_env_var() {
    let config = Config::default();
    let err = llm::build_llm_config_with_env(&config, |_| None)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("ANTHROPIC_API_KEY"),
        "error should mention env var fallback, got: {}",
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
    assert!(llm_config.model.contains("claude"), "default model should be claude, got: {}", llm_config.model);
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
    assert_eq!(llm_config.base_url.as_deref(), Some("http://localhost:8080"));
}

#[test]
fn test_system_prompt_is_concise() {
    assert!(llm::SYSTEM_PROMPT.contains("summarizing"));
    assert!(llm::SYSTEM_PROMPT.contains("3-5 sentences"));
}

// --- US-003: INSIGHTS_SYSTEM_PROMPT ---

#[test]
fn insights_system_prompt_requests_quantitative_insights() {
    let p = llm::INSIGHTS_SYSTEM_PROMPT;
    // Must ask for 4-6 insights
    assert!(p.contains("4") && p.contains("6"), "should specify 4-6 insights");
    // Each insight must start with quantitative claim
    assert!(p.to_lowercase().contains("quantitative") || p.to_lowercase().contains("specific number") || p.to_lowercase().contains("data reference"),
        "should require data-backed claims");
}

#[test]
fn insights_system_prompt_requires_bullet_format() {
    let p = llm::INSIGHTS_SYSTEM_PROMPT;
    assert!(p.to_lowercase().contains("bullet"), "should specify bullet point format");
    assert!(p.contains("1-2 sentence") || p.contains("1–2 sentence"), "should limit to 1-2 sentences per bullet");
}

#[test]
fn insights_system_prompt_prohibits_filler() {
    let p = llm::INSIGHTS_SYSTEM_PROMPT;
    let lower = p.to_lowercase();
    // Must explicitly prohibit generic advice/filler
    assert!(lower.contains("great job") || lower.contains("consider") || lower.contains("generic"),
        "should mention prohibited phrases");
}

#[test]
fn insights_system_prompt_under_200_words() {
    let words: Vec<&str> = llm::INSIGHTS_SYSTEM_PROMPT.split_whitespace().collect();
    assert!(words.len() <= 200, "system prompt has {} words, max 200", words.len());
}

#[test]
fn insights_system_prompt_distinct_from_summarize() {
    assert_ne!(llm::INSIGHTS_SYSTEM_PROMPT, llm::SYSTEM_PROMPT);
}

#[test]
fn test_build_llm_config_rejects_unsupported_provider_early() {
    // Regression: unsupported provider must fail at build time, not at
    // call_llm_streaming. Otherwise doctor/runtime diverge.
    let config = Config {
        llm_api_key: Some("whatever-key".to_string()),
        llm_provider: Some("gemini".to_string()),
        ..Config::default()
    };
    let err = llm::build_llm_config_with_env(&config, |_| None)
        .unwrap_err()
        .to_string();
    assert!(err.to_lowercase().contains("unsupported"));
    assert!(err.contains("gemini"));
    assert!(err.contains("anthropic") && err.contains("openai"),
        "error should list supported providers, got: {}", err);
}

#[test]
fn test_is_supported_provider_matches_runtime() {
    assert!(llm::is_supported_provider("anthropic"));
    assert!(llm::is_supported_provider("openai"));
    assert!(!llm::is_supported_provider("gemini"));
    assert!(!llm::is_supported_provider(""));
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
