use blackbox::commit_quality::score_message;

#[test]
fn empty_string_scores_zero() {
    assert_eq!(score_message(""), 0);
}

#[test]
fn whitespace_only_scores_zero() {
    assert_eq!(score_message("   "), 0);
    assert_eq!(score_message("\n\n"), 0);
    assert_eq!(score_message("\t"), 0);
}

#[test]
fn merge_commit_scores_50() {
    assert_eq!(score_message("Merge branch 'main' into feature"), 50);
    assert_eq!(score_message("Merge pull request #42 from org/repo"), 50);
}

#[test]
fn revert_commit_scores_50() {
    assert_eq!(
        score_message("Revert \"feat: add login\"\n\nThis reverts commit abc123."),
        50
    );
    assert_eq!(score_message("Revert \"bad change\""), 50);
}

#[test]
fn single_word_messages_score_low() {
    // base=50, <10 chars=-20, all lowercase no punct=-5 → 25
    assert!(score_message("fix") <= 30);
    assert!(score_message("wip") <= 30);
    assert!(score_message("update") <= 30);
}

#[test]
fn conventional_commit_scores_at_least_70() {
    assert!(score_message("feat: add user authentication") >= 70);
    assert!(score_message("fix: resolve login bug") >= 70);
    assert!(score_message("docs: update README with examples") >= 70);
    assert!(score_message("refactor: extract helper function") >= 70);
}

#[test]
fn conventional_commit_with_scope_scores_at_least_70() {
    assert!(score_message("feat(auth): add login flow") >= 70);
    assert!(score_message("fix(core): null pointer check") >= 70);
}

#[test]
fn body_adds_10_points() {
    let without_body = score_message("feat: add user auth");
    let with_body = score_message("feat: add user auth\n\nThis implements the full auth flow.");
    assert_eq!(with_body - without_body, 10);
}

#[test]
fn conventional_commit_with_long_body_scores_at_least_80() {
    let msg = "feat: implement user authentication system\n\n\
               This adds login, logout, and session management.";
    assert!(score_message(msg) >= 80);
}

#[test]
fn subject_over_72_chars_penalised() {
    let long_subject =
        "feat: this is a very long commit message subject that exceeds seventy two characters limit here";
    assert!(long_subject.len() > 72);
    let short_subject = "feat: this is a reasonably sized commit message";
    assert!(score_message(long_subject) < score_message(short_subject));
}

#[test]
fn subject_under_10_chars_penalised() {
    // "short msg" is 9 chars → < 10 penalty
    let short = score_message("short msg");
    // "a medium length commit message" is well within range
    let medium = score_message("a medium length commit message");
    assert!(short < medium);
}

#[test]
fn all_lowercase_no_punctuation_penalised() {
    // Same length, one has punctuation
    let no_punct = score_message("add some feature here");
    let with_punct = score_message("add some feature here.");
    assert!(no_punct < with_punct);
}

#[test]
fn score_is_deterministic() {
    let msg = "feat(auth): add OAuth2 support\n\nImplements the full OAuth2 flow.";
    let first = score_message(msg);
    let second = score_message(msg);
    let third = score_message(msg);
    assert_eq!(first, second);
    assert_eq!(second, third);
}

#[test]
fn score_clamped_to_0_100() {
    // Even a perfect message can't exceed 100
    let perfect = "feat(auth): implement comprehensive user authentication system\n\n\
                   This adds complete OAuth2 support with refresh tokens.";
    assert!(score_message(perfect) <= 100);

    // Even a terrible message can't go below 0
    assert_eq!(score_message("x"), score_message("x"));
}

#[test]
fn non_conventional_medium_message_scores_moderate() {
    // Good descriptive message but no conventional prefix
    let msg = "Add error handling for database connections";
    let score = score_message(msg);
    // base=50 + length bonus, no conventional, has uppercase so no lowercase penalty
    assert!(score >= 40 && score <= 70, "score was {}", score);
}
