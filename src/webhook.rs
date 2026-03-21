use serde_json::Value;

/// Build a Slack/Discord-compatible webhook body with a "text" key.
pub fn build_webhook_body(text: &str) -> Value {
    serde_json::json!({"text": text})
}

/// POST a JSON body to a webhook URL with 10s timeout.
/// Returns Ok(()) on 2xx, Err on failure.
pub fn send_webhook(url: &str, body: &Value) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let resp = client
        .post(url)
        .json(body)
        .send()
        .map_err(|e| format!("Webhook request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("Webhook returned HTTP {}", resp.status()))
    }
}

/// Convenience: build body and send to webhook URL.
/// Prints warning on failure, never panics.
pub fn post_to_webhook(url: &str, text: &str) {
    let body = build_webhook_body(text);
    if let Err(e) = send_webhook(url, &body) {
        eprintln!("Warning: webhook delivery failed: {e}");
    }
}
