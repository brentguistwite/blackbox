use serde_json::json;

#[test]
fn test_build_webhook_body_simple_text() {
    let body = blackbox::webhook::build_webhook_body("hello");
    assert_eq!(body, json!({"text": "hello"}));
}

#[test]
fn test_build_webhook_body_multiline() {
    let text = "**Today (Mar 21)**\n\n- repo1 (~30m)\n  - main: 5 commits";
    let body = blackbox::webhook::build_webhook_body(text);
    assert_eq!(body["text"].as_str().unwrap(), text);
}

#[test]
fn test_build_webhook_body_empty() {
    let body = blackbox::webhook::build_webhook_body("");
    assert_eq!(body, json!({"text": ""}));
}
