use anyhow::{bail, Context};
use std::io::{BufRead, Write};
use std::time::Duration;

use crate::config::Config;

#[derive(Debug)]
pub struct LlmConfig {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
}

pub const SYSTEM_PROMPT: &str = "You are summarizing a developer's work activity. \
Be concise, highlight key accomplishments, mention repos and PRs by name. \
Write 3-5 sentences.";

/// Build LlmConfig from Config, returning helpful error if API key missing.
pub fn build_llm_config(config: &Config) -> anyhow::Result<LlmConfig> {
    let api_key = config.llm_api_key.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "No LLM API key configured. Add to ~/.config/blackbox/config.toml:\n\n\
             llm_api_key = \"your-api-key\"\n\
             llm_provider = \"anthropic\"  # or \"openai\"\n\
             llm_model = \"claude-sonnet-4-20250514\"  # optional"
        )
    })?;

    let provider = config
        .llm_provider
        .as_deref()
        .unwrap_or("anthropic")
        .to_string();

    let model = config.llm_model.clone().unwrap_or_else(|| match provider.as_str() {
        "anthropic" => "claude-sonnet-4-20250514".to_string(),
        _ => "gpt-4o-mini".to_string(),
    });

    Ok(LlmConfig {
        provider,
        api_key: api_key.clone(),
        model,
        base_url: config.llm_base_url.clone(),
    })
}

/// Stream an LLM summary of activity JSON to stdout.
pub fn summarize_activity(config: &LlmConfig, activity_json: &str) -> anyhow::Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    match config.provider.as_str() {
        "anthropic" => stream_anthropic(&client, config, activity_json),
        "openai" => stream_openai(&client, config, activity_json),
        other => bail!(
            "Unsupported LLM provider: '{}'. Use 'anthropic' or 'openai'.",
            other
        ),
    }
}

fn stream_anthropic(
    client: &reqwest::blocking::Client,
    config: &LlmConfig,
    activity_json: &str,
) -> anyhow::Result<()> {
    let url = "https://api.anthropic.com/v1/messages";
    let body = serde_json::json!({
        "model": config.model,
        "max_tokens": 1024,
        "stream": true,
        "system": SYSTEM_PROMPT,
        "messages": [{
            "role": "user",
            "content": format!("Summarize this developer activity:\n\n{}", activity_json)
        }]
    });

    let resp = client
        .post(url)
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .context("Failed to connect to Anthropic API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        bail!("Anthropic API error ({}): {}", status, text);
    }

    parse_sse_stream(resp, |event| {
        if event["type"] == "content_block_delta" {
            event["delta"]["text"].as_str().map(|s| s.to_string())
        } else {
            None
        }
    })
}

fn stream_openai(
    client: &reqwest::blocking::Client,
    config: &LlmConfig,
    activity_json: &str,
) -> anyhow::Result<()> {
    let base_url = config
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com");
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": config.model,
        "max_tokens": 1024,
        "stream": true,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": format!("Summarize this developer activity:\n\n{}", activity_json)}
        ]
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .context("Failed to connect to OpenAI-compatible API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        bail!("OpenAI API error ({}): {}", status, text);
    }

    parse_sse_stream(resp, |event| {
        event["choices"][0]["delta"]["content"]
            .as_str()
            .map(|s| s.to_string())
    })
}

/// Parse SSE stream, calling extractor on each data event to get text chunks.
fn parse_sse_stream(
    resp: reqwest::blocking::Response,
    extractor: impl Fn(&serde_json::Value) -> Option<String>,
) -> anyhow::Result<()> {
    let reader = std::io::BufReader::new(resp);
    let mut stdout = std::io::stdout();

    for line in reader.lines() {
        let line = line?;
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" {
                break;
            }
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data)
                && let Some(text) = extractor(&event)
            {
                print!("{}", text);
                stdout.flush()?;
            }
        }
    }
    println!();
    Ok(())
}
