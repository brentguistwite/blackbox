use blackbox::enrichment::PrInfo;

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
