const CONVENTIONAL_TYPES: &[&str] = &[
    "feat", "fix", "chore", "docs", "refactor", "test", "style", "perf", "ci", "build", "revert",
];

/// Score a commit message on a 0–100 scale.
pub fn score_message(msg: &str) -> u8 {
    let trimmed = msg.trim();

    if trimmed.is_empty() {
        return 0;
    }

    // Merge and revert commits: fixed 50
    if trimmed.starts_with("Merge ") || trimmed.starts_with("Revert ") {
        return 50;
    }

    let (subject, has_body) = match trimmed.find("\n\n") {
        Some(pos) => (trimmed[..pos].trim(), true),
        None => (trimmed.lines().next().unwrap_or("").trim(), false),
    };

    let subject_len = subject.len();
    let mut score: i32 = 50;

    // +up to 30 for subject length 10–72 (proportional)
    if (10..=72).contains(&subject_len) {
        score += ((subject_len - 10) as f64 / 62.0 * 30.0) as i32;
    }

    // +20 conventional commit prefix
    if is_conventional_commit(subject) {
        score += 20;
    }

    // +10 body present
    if has_body {
        score += 10;
    }

    // -20 subject < 10 chars
    if subject_len < 10 {
        score -= 20;
    }

    // -10 subject > 72 chars
    if subject_len > 72 {
        score -= 10;
    }

    // -5 all lowercase with no punctuation
    if is_all_lowercase_no_punctuation(subject) {
        score -= 5;
    }

    score.clamp(0, 100) as u8
}

fn is_conventional_commit(subject: &str) -> bool {
    for t in CONVENTIONAL_TYPES {
        let rest = match subject.strip_prefix(t) {
            Some(r) => r,
            None => continue,
        };
        // type: description
        if rest.starts_with(": ") {
            return true;
        }
        // type(scope): description
        if rest.starts_with('(')
            && rest.find("): ").is_some_and(|close| close > 1)
        {
            return true;
        }
    }
    false
}

fn is_all_lowercase_no_punctuation(subject: &str) -> bool {
    if subject.is_empty() {
        return false;
    }
    !subject.chars().any(|c| c.is_uppercase() || c.is_ascii_punctuation())
}
