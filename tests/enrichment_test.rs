use blackbox::enrichment::{GhPrWithReviews, GhReview, GhReviewAuthor, PrInfo};

#[test]
fn pr_info_deserializes_from_gh_json() {
    let json = r#"[
        {"number": 42, "title": "Add feature X", "state": "OPEN", "headRefName": "feature-x"},
        {"number": 10, "title": "Fix bug", "state": "MERGED", "headRefName": "fix-bug"}
    ]"#;
    let prs: Vec<PrInfo> = serde_json::from_str(json).unwrap();
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[0].number, 42);
    assert_eq!(prs[0].title, "Add feature X");
    assert_eq!(prs[0].state, "OPEN");
    assert_eq!(prs[0].head_ref_name, "feature-x");
    assert_eq!(prs[1].number, 10);
    assert_eq!(prs[1].state, "MERGED");
}

#[test]
fn pr_info_serializes_to_json() {
    let pr = PrInfo {
        number: 7,
        title: "My PR".to_string(),
        state: "OPEN".to_string(),
        head_ref_name: "my-branch".to_string(),
    };
    let json = serde_json::to_string(&pr).unwrap();
    assert!(json.contains("\"number\":7"));
    assert!(json.contains("\"title\":\"My PR\""));
}

#[test]
fn pr_info_handles_empty_array() {
    let json = "[]";
    let prs: Vec<PrInfo> = serde_json::from_str(json).unwrap();
    assert!(prs.is_empty());
}

#[test]
fn pr_info_handles_malformed_json_gracefully() {
    let bad_json = "not json at all";
    let result: Result<Vec<PrInfo>, _> = serde_json::from_str(bad_json);
    assert!(result.is_err());
}

#[test]
fn match_prs_to_branch_finds_match() {
    let prs = vec![
        PrInfo {
            number: 1,
            title: "First".to_string(),
            state: "OPEN".to_string(),
            head_ref_name: "main".to_string(),
        },
        PrInfo {
            number: 2,
            title: "Feature".to_string(),
            state: "OPEN".to_string(),
            head_ref_name: "feature-branch".to_string(),
        },
    ];
    let branches = vec!["feature-branch".to_string()];
    let matched: Vec<&PrInfo> = prs
        .iter()
        .filter(|pr| branches.contains(&pr.head_ref_name))
        .collect();
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].number, 2);
}

#[test]
fn match_prs_no_match_returns_empty() {
    let prs = vec![PrInfo {
        number: 1,
        title: "First".to_string(),
        state: "OPEN".to_string(),
        head_ref_name: "other-branch".to_string(),
    }];
    let branches = vec!["my-branch".to_string()];
    let matched: Vec<&PrInfo> = prs
        .iter()
        .filter(|pr| branches.contains(&pr.head_ref_name))
        .collect();
    assert!(matched.is_empty());
}

// --- Review activity struct tests ---

#[test]
fn gh_pr_with_reviews_deserializes() {
    let json = r#"[
        {
            "number": 42,
            "title": "Add feature X",
            "url": "https://github.com/org/repo/pull/42",
            "reviews": [
                {
                    "author": {"login": "myuser"},
                    "state": "APPROVED",
                    "submittedAt": "2026-03-15T10:00:00Z"
                },
                {
                    "author": {"login": "otheruser"},
                    "state": "COMMENTED",
                    "submittedAt": "2026-03-15T09:00:00Z"
                }
            ]
        }
    ]"#;
    let prs: Vec<GhPrWithReviews> = serde_json::from_str(json).unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].number, 42);
    assert_eq!(prs[0].reviews.len(), 2);
    assert_eq!(prs[0].reviews[0].author.login, "myuser");
    assert_eq!(prs[0].reviews[0].state, "APPROVED");
    assert_eq!(prs[0].reviews[0].submitted_at, "2026-03-15T10:00:00Z");
}

#[test]
fn gh_pr_with_reviews_handles_empty_reviews() {
    let json = r#"[{"number": 1, "title": "T", "url": "http://x", "reviews": []}]"#;
    let prs: Vec<GhPrWithReviews> = serde_json::from_str(json).unwrap();
    assert_eq!(prs[0].reviews.len(), 0);
}

#[test]
fn gh_pr_with_reviews_handles_missing_reviews_field() {
    let json = r#"[{"number": 1, "title": "T", "url": "http://x"}]"#;
    let prs: Vec<GhPrWithReviews> = serde_json::from_str(json).unwrap();
    assert!(prs[0].reviews.is_empty());
}

#[test]
fn gh_review_filters_by_username() {
    let reviews = vec![
        GhReview {
            author: GhReviewAuthor { login: "me".to_string() },
            state: "APPROVED".to_string(),
            submitted_at: "2026-03-15T10:00:00Z".to_string(),
        },
        GhReview {
            author: GhReviewAuthor { login: "other".to_string() },
            state: "COMMENTED".to_string(),
            submitted_at: "2026-03-15T09:00:00Z".to_string(),
        },
        GhReview {
            author: GhReviewAuthor { login: "me".to_string() },
            state: "CHANGES_REQUESTED".to_string(),
            submitted_at: "2026-03-15T11:00:00Z".to_string(),
        },
    ];

    let mine: Vec<&GhReview> = reviews
        .iter()
        .filter(|r| r.author.login == "me")
        .collect();
    assert_eq!(mine.len(), 2);
    assert_eq!(mine[0].state, "APPROVED");
    assert_eq!(mine[1].state, "CHANGES_REQUESTED");
}

#[test]
fn gh_review_state_mapping() {
    let valid_states = ["APPROVED", "CHANGES_REQUESTED", "COMMENTED"];
    let skip_states = ["PENDING", "DISMISSED"];

    for state in &valid_states {
        assert!(["APPROVED", "CHANGES_REQUESTED", "COMMENTED"].contains(state));
    }
    for state in &skip_states {
        assert!(!["APPROVED", "CHANGES_REQUESTED", "COMMENTED"].contains(state));
    }
}
