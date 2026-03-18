use blackbox::setup::{
    detect_shell_type_from, hook_comment_block, notset_shell_message, unsupported_shell_message,
    ShellDetection,
};

// --- detect_shell_type_from tests ---

#[test]
fn detect_shell_zsh_returns_supported() {
    let result = detect_shell_type_from(Some("/bin/zsh"));
    match result {
        ShellDetection::Supported { name, rc_path } => {
            assert_eq!(name, "zsh");
            assert!(rc_path.ends_with(".zshrc"), "rc_path should end with .zshrc, got {:?}", rc_path);
        }
        other => panic!("expected Supported, got {:?}", other),
    }
}

#[test]
fn detect_shell_bash_returns_supported() {
    let result = detect_shell_type_from(Some("/bin/bash"));
    match result {
        ShellDetection::Supported { name, rc_path } => {
            assert_eq!(name, "bash");
            assert!(rc_path.ends_with(".bashrc"), "rc_path should end with .bashrc, got {:?}", rc_path);
        }
        other => panic!("expected Supported, got {:?}", other),
    }
}

#[test]
fn detect_shell_fish_returns_supported() {
    let result = detect_shell_type_from(Some("/usr/bin/fish"));
    match result {
        ShellDetection::Supported { name, rc_path } => {
            assert_eq!(name, "fish");
            assert!(
                rc_path.ends_with("config.fish"),
                "rc_path should end with config.fish, got {:?}",
                rc_path
            );
        }
        other => panic!("expected Supported, got {:?}", other),
    }
}

#[test]
fn detect_shell_nushell_returns_unsupported() {
    let result = detect_shell_type_from(Some("/usr/bin/nushell"));
    match result {
        ShellDetection::Unsupported(name) => assert_eq!(name, "nushell"),
        other => panic!("expected Unsupported, got {:?}", other),
    }
}

#[test]
fn detect_shell_unset_returns_notset() {
    let result = detect_shell_type_from(None);
    assert!(matches!(result, ShellDetection::NotSet));
}

#[test]
fn detect_shell_empty_returns_notset() {
    let result = detect_shell_type_from(Some(""));
    assert!(matches!(result, ShellDetection::NotSet));
}

// --- rc file comment tests ---

#[test]
fn hook_comment_contains_time_estimation() {
    let comment = hook_comment_block("zsh");
    assert!(comment.contains("time estimation"), "comment should mention time estimation");
}

#[test]
fn hook_comment_contains_disable() {
    let comment = hook_comment_block("zsh");
    assert!(comment.contains("disable"), "comment should mention how to disable");
}

#[test]
fn hook_comment_contains_eval_line() {
    let comment = hook_comment_block("bash");
    assert!(
        comment.contains("eval \"$(blackbox hook bash)\""),
        "comment should contain the eval line for the shell"
    );
}

// --- unsupported shell message tests ---

#[test]
fn unsupported_message_contains_shell_name() {
    let msg = unsupported_shell_message("nushell");
    assert!(msg.contains("nushell"), "message should contain detected shell name");
}

#[test]
fn unsupported_message_contains_manual_instructions() {
    let msg = unsupported_shell_message("nushell");
    assert!(msg.contains("eval"), "message should contain manual eval instruction");
    assert!(msg.contains("zsh, bash, and fish") || msg.contains("zsh/bash/fish"),
        "message should list supported shells");
}

// --- notset shell message tests ---

#[test]
fn notset_message_mentions_shell_var() {
    let msg = notset_shell_message();
    assert!(msg.contains("$SHELL"), "message should mention $SHELL");
}

#[test]
fn notset_message_contains_manual_instructions() {
    let msg = notset_shell_message();
    assert!(msg.contains("eval"), "message should contain manual eval instruction");
}
