use anyhow::{bail, Result};

/// Generate a shell hook script for the given shell.
/// The script calls `blackbox _notify-dir $PWD` on directory change.
pub fn generate_hook(shell: &str) -> Result<String> {
    match shell {
        "zsh" => Ok(ZSH_HOOK.to_string()),
        "bash" => Ok(BASH_HOOK.to_string()),
        "fish" => Ok(FISH_HOOK.to_string()),
        other => bail!("Unsupported shell: {}. Supported: zsh, bash, fish", other),
    }
}

const ZSH_HOOK: &str = r#"_blackbox_hook() {
  blackbox _notify-dir "$PWD" &>/dev/null &!
}
chpwd_functions+=(_blackbox_hook)
"#;

const BASH_HOOK: &str = r#"_blackbox_hook() {
  blackbox _notify-dir "$PWD" &>/dev/null &
}
if [[ -z "$_BLACKBOX_HOOKED" ]]; then
  _BLACKBOX_HOOKED=1
  PROMPT_COMMAND="_blackbox_hook;${PROMPT_COMMAND:-}"
fi
"#;

const FISH_HOOK: &str = r#"function _blackbox_hook --on-variable PWD
  blackbox _notify-dir $PWD &>/dev/null &
end
"#;
