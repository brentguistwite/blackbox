use blackbox::shell_hook::generate_hook;

#[test]
fn zsh_hook_contains_chpwd() {
    let script = generate_hook("zsh").unwrap();
    assert!(script.contains("chpwd_functions"), "zsh hook must use chpwd_functions");
    assert!(script.contains("_blackbox_hook"), "zsh hook must define _blackbox_hook");
    assert!(script.contains("_notify-dir"), "zsh hook must call _notify-dir");
}

#[test]
fn bash_hook_contains_prompt_command() {
    let script = generate_hook("bash").unwrap();
    assert!(script.contains("PROMPT_COMMAND"), "bash hook must use PROMPT_COMMAND");
    assert!(script.contains("_BLACKBOX_HOOKED"), "bash hook must guard against double-hook");
    assert!(script.contains("_notify-dir"), "bash hook must call _notify-dir");
}

#[test]
fn bash_hook_preserves_existing_prompt_command() {
    let script = generate_hook("bash").unwrap();
    // Must not clobber: should reference existing PROMPT_COMMAND
    assert!(
        script.contains("${PROMPT_COMMAND") || script.contains("$PROMPT_COMMAND"),
        "bash hook must preserve existing PROMPT_COMMAND"
    );
}

#[test]
fn fish_hook_uses_on_variable() {
    let script = generate_hook("fish").unwrap();
    assert!(script.contains("--on-variable PWD"), "fish hook must use --on-variable PWD");
    assert!(script.contains("_notify-dir"), "fish hook must call _notify-dir");
    // Fish must NOT use bash-isms
    assert!(!script.contains("$()"), "fish hook must not use $() syntax");
    assert!(!script.contains("export "), "fish hook must not use export");
}

#[test]
fn unknown_shell_returns_error() {
    let result = generate_hook("powershell");
    assert!(result.is_err(), "unknown shell should return error");
}

#[test]
fn all_hooks_contain_blackbox_binary() {
    for shell in &["zsh", "bash", "fish"] {
        let script = generate_hook(shell).unwrap();
        assert!(
            script.contains("blackbox"),
            "{} hook must reference blackbox binary",
            shell
        );
    }
}
