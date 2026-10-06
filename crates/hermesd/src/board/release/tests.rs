//! The gate's pure parts: the §6.1 outcomes and the frozen hash.

use chrono::Utc;

use super::model::{Release, ReleaseBuild, ReleaseItem, ReleaseStatus, ReleaseTest, Verdict};
use super::rule::{outcome, Outcome};
use super::{check_frozen, frozen_hash};

#[test]
fn verdicts_decide_the_outcome() {
    use Verdict::*;
    let cases: &[(&[Verdict], Outcome)] = &[
        (&[Ship, Ship], Outcome::Approved),
        (&[Hold, Hold], Outcome::Held),
        (&[Ship, Hold], Outcome::Repackaging),
        (&[Ship, Rework], Outcome::Repackaging),
        (&[Rework, Rework], Outcome::Rejected),
        (&[Hold, Rework], Outcome::Rejected),
    ];
    for (verdicts, want) in cases {
        assert_eq!(outcome(verdicts), *want, "{verdicts:?}");
    }
}

fn release() -> Release {
    let now = Utc::now();
    Release {
        id: "r1".into(),
        project_id: "p".into(),
        name: "0.16.0".into(),
        display_version: None,
        status: ReleaseStatus::AwaitingOwner,
        decision_id: None,
        supersedes: None,
        install_mode: "side_by_side".into(),
        rollback_to: None,
        changelog: String::new(),
        how_to_test: serde_json::json!([]),
        frozen_at: None,
        frozen_hash: None,
        created_by: "ops".into(),
        created_at: now,
        updated_at: now,
        version: 1,
        paused_reason: None,
        held_note: None,
        remind_at: None,
        items: ["H-2", "H-1"]
            .iter()
            .map(|id| ReleaseItem {
                item_id: id.to_string(),
                verdict: Verdict::Pending,
                owner_note: None,
            })
            .collect(),
        builds: vec![ReleaseBuild {
            platform: "desktop-mac".into(),
            version: "0.16.0".into(),
            artifact: "/builds/The Hermes.dmg".into(),
            url: None,
            install_url: None,
            sha256: "a".repeat(64),
            built_at: now,
            source_commit: None,
        }],
        tests: Vec::new(),
        deployments: Vec::new(),
        events: Vec::new(),
        post_install: Vec::new(),
    }
}

#[test]
fn the_hash_pins_items_builds_and_tests_not_verdicts_or_order() {
    let mut r = release();
    let hash = frozen_hash(&r);
    r.frozen_hash = Some(hash.clone());
    assert!(check_frozen(&r).is_ok());

    // Verdicts and item order are not the package's content.
    r.items.reverse();
    r.items[0].verdict = Verdict::Ship;
    assert_eq!(frozen_hash(&r), hash);

    let mut rebuilt = r.clone();
    rebuilt.builds[0].sha256 = "b".repeat(64);
    assert!(
        check_frozen(&rebuilt).is_err(),
        "a changed build needs a resubmit"
    );

    let mut retested = r.clone();
    retested.tests.push(ReleaseTest {
        machine: "imac".into(),
        tester: "t".into(),
        build_sha256: "a".repeat(64),
        result: "pass".into(),
    });
    assert!(check_frozen(&retested).is_err());

    let mut fewer = r.clone();
    fewer.items.pop();
    assert!(check_frozen(&fewer).is_err());

    let mut unsubmitted = release();
    unsubmitted.frozen_hash = None;
    assert!(check_frozen(&unsubmitted).is_err());
}
