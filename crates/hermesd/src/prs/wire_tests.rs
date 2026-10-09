//! The JSON `pr_get` gives bots, as `hermes.pr.v1` messages.

use serde_json::json;

use super::*;

struct Stub;

impl Names for Stub {
    fn daemon_id(&self) -> String {
        "mac".into()
    }
    fn bot_name(&self, id: &str) -> Option<String> {
        (id == "dev").then(|| "Desktop Dev".into())
    }
    fn device_name(&self, id: &str) -> Option<String> {
        (id == "d1").then(|| "iPhone".into())
    }
    fn item_title(&self, id: &str) -> Option<String> {
        (id == "H-1").then(|| "Search".into())
    }
}

/// AC3: a log kept on a linked computer is `<computer>:<path>`, text that
/// names a file over there. It comes through as it is, whether or not this
/// disk has such a file, and nothing is read from it.
#[test]
fn a_linked_computers_log_ref_is_opaque_text() {
    let dir = tempfile::tempdir().unwrap();
    let here = dir.path().join("check.log");
    std::fs::write(&here, "SECRET local file\n").unwrap();
    for log in [
        format!("win:{}", here.display()),
        "win:C:\\Users\\t\\run\\checks\\j1\\check.log".to_string(),
        "imac:/nowhere/check.log".to_string(),
        "checks/0123456789ab-rust.log".to_string(),
    ] {
        let run = check(
            &Stub,
            &json!({"name": "rust", "result": "pass", "log_url": log}),
        );
        assert_eq!(run.log_url, log);
        let bytes = format!("{run:?}");
        assert!(!bytes.contains("SECRET"), "{bytes}");
    }
}

#[test]
fn a_pr_maps_its_names_to_wire_enums_and_refs() {
    let pr = pull_request(
        &Stub,
        &json!({
            "id": "p1", "project_id": "x", "number": 3, "item_id": "H-1", "state": "merging",
            "author": "dev", "head_sha": "abc", "opened_at": "2026-10-09T10:00:00+00:00",
            "required_roles": ["architect"], "comment_count": 2, "moved_unreported": true,
            "merge": {"merge_at": "2026-10-09T10:00:10+00:00"},
            "reviews": [
                {"role": "architect", "reviewer": "dev", "verdict": "changes_requested",
                 "findings": [{"severity": "must", "text": "Fix", "line": 4}], "stale": true},
                {"role": "owner", "reviewer": "owner", "provenance": "device:d1",
                 "verdict": "approved"},
            ],
            "checks": [{"name": "rust", "result": "running", "runner": "daemon:check-runner",
                        "tool_versions": {"cargo": "1.99.0"}}],
            "mergeable": {"ok": false, "blockers": [
                {"kind": "unresolved_must", "text": "1 must", "subject": "comments", "paths": []},
                {"kind": "conflicts", "text": "c", "subject": null, "paths": ["a.rs"]},
            ]},
        }),
    );
    assert_eq!(pr.state, p::PrState::Merging as i32);
    assert_eq!(pr.item_title, "Search");
    assert_eq!(pr.author.as_ref().unwrap().name, "Desktop Dev");
    assert_eq!(pr.author.as_ref().unwrap().daemon_id, "mac");
    assert_eq!(pr.opened_at.unwrap().seconds, 1_791_540_000);
    assert_eq!(pr.merge_at.unwrap().seconds, 1_791_540_010);
    assert_eq!((pr.comment_count, pr.moved_unreported), (2, true));
    let [arch, owner] = &pr.reviews[..] else {
        panic!("two reviews");
    };
    assert_eq!(arch.verdict, p::Verdict::ChangesRequested as i32);
    assert_eq!(arch.findings[0].severity, p::Severity::Must as i32);
    assert!(arch.stale);
    let Some(p::reviewer::Who::Owner(o)) = owner.reviewer.clone().unwrap().who else {
        panic!("the owner");
    };
    assert_eq!(
        (o.device_id.as_str(), o.device_name.as_str()),
        ("d1", "iPhone")
    );
    assert_eq!(pr.checks[0].result, p::CheckResult::Running as i32);
    assert_eq!(
        pr.checks[0].runner.as_ref().unwrap().name,
        "daemon:check-runner"
    );
    assert_eq!(pr.checks[0].tool_versions["cargo"], "1.99.0");
    let kinds: Vec<i32> = pr
        .mergeable
        .unwrap()
        .blockers
        .iter()
        .map(|b| b.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            p::BlockerKind::UnresolvedMust as i32,
            p::BlockerKind::Conflicts as i32
        ]
    );
}

#[test]
fn a_comment_shows_at_its_line_there_or_where_it_was_written() {
    let shown = comment(
        &Stub,
        &json!({"id": "c", "line": 4, "shown_line": 6, "side": "new", "author": "owner:ticket",
                "severity": "should", "outdated": false}),
    );
    assert_eq!((shown.line, shown.side), (6, p::Side::New as i32));
    assert_eq!(shown.severity, p::Severity::Should as i32);
    assert!(matches!(
        shown.author.unwrap().who,
        Some(p::reviewer::Who::Owner(p::Owner { ref device_id, .. })) if device_id.is_empty()
    ));
    let gone = comment(
        &Stub,
        &json!({"id": "c", "line": 4, "shown_line": null, "outdated": true, "author": "dev"}),
    );
    assert_eq!((gone.line, gone.outdated), (4, true));
}
