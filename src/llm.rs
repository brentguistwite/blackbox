use anyhow::{bail, Context};
use std::io::{BufRead, Write};
use std::time::Duration;

use crate::config::Config;
use crate::query::{InsightsData, RepoInsights};

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

pub const INSIGHTS_SYSTEM_PROMPT: &str = "\
You are analyzing a developer's work patterns from quantitative activity data. \
Produce exactly 4-6 behavioral insights as a bullet list. Each bullet must: \
(1) open with a specific data reference citing numbers from the input \
(e.g. \"Tuesdays account for 28% of your commits\", \"Your Friday messages average 12 chars shorter\"), \
(2) be 1-2 sentences, and (3) describe a pattern, not prescribe an action. \
Do not use headers, sub-lists, or section breaks. \
Do not use filler, generic advice, evaluative praise, or recommendation language. \
Report observable patterns only.";

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
    call_llm_streaming(
        config,
        SYSTEM_PROMPT,
        &format!("Summarize this developer activity:\n\n{}", activity_json),
        1024,
        30,
    )
}

/// Stream LLM-generated behavioral insights to stdout.
pub fn generate_insights(config: &LlmConfig, prompt: &str, max_tokens: u32) -> anyhow::Result<()> {
    call_llm_streaming(config, INSIGHTS_SYSTEM_PROMPT, prompt, max_tokens, 60)
}

/// Core streaming helper — routes to Anthropic/OpenAI, handles 429/errors, streams to stdout.
pub fn call_llm_streaming(
    config: &LlmConfig,
    system_prompt: &str,
    user_content: &str,
    max_tokens: u32,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()?;

    match config.provider.as_str() {
        "anthropic" => {
            let url = "https://api.anthropic.com/v1/messages";
            let body = serde_json::json!({
                "model": config.model,
                "max_tokens": max_tokens,
                "stream": true,
                "system": system_prompt,
                "messages": [{"role": "user", "content": user_content}]
            });

            let resp = client
                .post(url)
                .header("x-api-key", &config.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .context("Failed to connect to Anthropic API")?;

            if resp.status() == 429 {
                bail!("Rate limited by Anthropic. Try again in a moment.");
            }
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
        "openai" => {
            let base_url = config
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com");
            let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

            let body = serde_json::json!({
                "model": config.model,
                "max_tokens": max_tokens,
                "stream": true,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": user_content}
                ]
            });

            let resp = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", config.api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .context("Failed to connect to OpenAI-compatible API")?;

            if resp.status() == 429 {
                bail!("Rate limited by OpenAI. Try again in a moment.");
            }
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
        other => bail!(
            "Unsupported LLM provider: '{}'. Use 'anthropic' or 'openai'.",
            other
        ),
    }
}

// --- US-002: Insights prompt construction ---

fn truncate_repos_for_prompt(repos: &[RepoInsights], max: usize) -> (&[RepoInsights], bool) {
    if repos.len() <= max {
        (repos, false)
    } else {
        (&repos[..max], true)
    }
}

/// Build a compact text prompt from InsightsData for the LLM.
pub fn build_insights_prompt(data: &InsightsData) -> String {
    let mut out = String::with_capacity(2048);

    out.push_str(
        "Analyze these developer activity patterns and provide specific, data-driven behavioral insights:\n\n",
    );

    // Period + totals
    out.push_str(&format!(
        "Period: {}\nTotal commits: {} across {} repos\n\n",
        data.period_label, data.total_commits, data.total_repos
    ));

    // Commits by DOW with percentages
    let dow_names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let total_dow: u32 = data.commits_by_dow.iter().sum();
    if total_dow > 0 {
        out.push_str("Commits by day: ");
        let parts: Vec<String> = dow_names
            .iter()
            .zip(data.commits_by_dow.iter())
            .filter(|(_, count)| **count > 0)
            .map(|(name, &count)| {
                let pct = (count as f64 / total_dow as f64 * 100.0).round() as u32;
                format!("{}: {}% ({})", name, pct, count)
            })
            .collect();
        out.push_str(&parts.join(", "));
        out.push('\n');
    }

    // Peak commit hour + top 3
    let total_hour: u32 = data.commits_by_hour.iter().sum();
    if total_hour > 0 {
        let mut hour_indexed: Vec<(usize, u32)> = data
            .commits_by_hour
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(i, &c)| (i, c))
            .collect();
        hour_indexed.sort_by(|a, b| b.1.cmp(&a.1));
        if let Some(&(peak_h, peak_c)) = hour_indexed.first() {
            let top3: Vec<String> = hour_indexed.iter().take(3).map(|(h, _)| format!("{}", h)).collect();
            out.push_str(&format!(
                "Peak commit hour: {}:00 ({} commits); top hours: {}\n",
                peak_h, peak_c, top3.join(", ")
            ));
        }
    }

    // Bug-fix ratio
    if data.total_commits > 0 {
        let pct = (data.bugfix_commits as f64 / data.total_commits as f64 * 100.0).round() as u32;
        out.push_str(&format!(
            "Bug-fix commits: {}/{} ({}%)\n",
            data.bugfix_commits, data.total_commits, pct
        ));
    }

    // Avg commit msg length by DOW
    let days_with_msgs: Vec<(usize, f64)> = data
        .avg_msg_len_by_dow
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0.0)
        .map(|(i, &v)| (i, v))
        .collect();
    if days_with_msgs.len() >= 2 {
        out.push_str("Avg commit msg length by day: ");
        let parts: Vec<String> = days_with_msgs
            .iter()
            .map(|(i, v)| format!("{}: {}ch", dow_names[*i], v.round() as i64))
            .collect();
        out.push_str(&parts.join(", "));

        // Trend annotation: compare Mon (index 0) vs Fri (index 4) if both have data
        let mon_avg = data.avg_msg_len_by_dow[0];
        let fri_avg = data.avg_msg_len_by_dow[4];
        if mon_avg > 0.0 && fri_avg > 0.0 {
            if mon_avg > fri_avg + 10.0 {
                out.push_str(" (trend: shorter toward end of week)");
            } else if fri_avg > mon_avg + 10.0 {
                out.push_str(" (trend: longer toward end of week)");
            }
        }
        out.push('\n');
    }

    // PR merge times (only if non-empty)
    if !data.pr_merge_times_hours.is_empty() {
        let mut sorted = data.pr_merge_times_hours.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];
        let min = sorted.first().unwrap();
        let max = sorted.last().unwrap();
        out.push_str(&format!(
            "PR merge times: median {:.1}h, range {:.1}\u{2013}{:.1}h ({} PRs)\n",
            median, min, max, sorted.len()
        ));
    }

    out.push('\n');

    // Per-repo breakdown (sorted by commits desc, capped at 10, then trimmed to fit 8000 chars)
    let mut sorted_repos = data.per_repo.clone();
    sorted_repos.sort_by(|a, b| b.commits.cmp(&a.commits));

    let mut max_repos = 10;
    loop {
        let (visible, truncated) = truncate_repos_for_prompt(&sorted_repos, max_repos);
        let mut repo_section = String::from("Top repos:\n");
        for r in visible {
            let hours = r.estimated_minutes / 60;
            let mins = r.estimated_minutes % 60;
            let pr_marker = if r.has_prs { " [has PRs]" } else { "" };
            repo_section.push_str(&format!(
                "- {}: {} commits, ~{}h {}m, {} branches{}\n",
                r.repo_name, r.commits, hours, mins, r.branches_touched, pr_marker
            ));
        }
        if truncated || max_repos < sorted_repos.len() {
            repo_section.push_str(&format!(
                "(showing top {} of {} repos)\n",
                visible.len(),
                data.total_repos
            ));
        }

        if out.len() + repo_section.len() <= 8000 || max_repos <= 1 {
            out.push_str(&repo_section);
            break;
        }
        max_repos -= 1;
    }

    out
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
