//! Acceptance criteria over MCP (ARCH-R8 F1): a bot adds and edits them
//! with `item_update`, so an item created without them can still meet the
//! Definition of Ready.

mod common;

use common::board::version;
use common::tasks::{error_text, project_with_bots};
use serde_json::json;

#[tokio::test]
async fn bots_add_and_edit_acceptance_criteria_to_meet_the_dor() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "Tester"]).await;
    let [lead, tester] = &mut bots[..] else {
        unreachable!()
    };
    let project = pair
        .d
        .app
        .db
        .get_bot(&pair.ids[0])
        .unwrap()
        .unwrap()
        .project_id;
    pair.d
        .app
        .db
        .ensure_board(&project, "d-test", Some("H"))
        .unwrap();

    // Created without criteria, it can't be refined...
    let created = lead
        .call(
            "item_create",
            json!({"type": "feature", "title": "No AC", "platforms": ["infra"], "size": "S"}),
        )
        .await;
    let id = created["item"]["id"].as_str().unwrap().to_string();
    let item = created["item"].clone();
    let raw = lead
        .call_raw(
            "item_move",
            json!({"id": id, "to": "ready", "expected_version": version(&item)}),
        )
        .await;
    assert!(
        error_text(&raw).contains("dor.acceptance_criteria"),
        "{raw}"
    );

    // ...until a bot adds them with item_update.
    let updated = lead
        .call(
            "item_update",
            json!({"id": id, "expected_version": version(&item),
                   "acceptance_criteria": ["builds", "tests pass"]}),
        )
        .await;
    let ac = &updated["item"]["acceptance_criteria"];
    assert_eq!(
        (ac[0]["text"].as_str(), ac[1]["text"].as_str()),
        (Some("builds"), Some("tests pass"))
    );
    let checked = tester
        .call(
            "item_check_ac",
            json!({"id": id, "expected_version": version(&updated["item"]), "index": 0, "result": "pass"}),
        )
        .await;

    // Editing the list keeps the check on an unchanged criterion.
    let edited = lead
        .call(
            "item_update",
            json!({"id": id, "expected_version": version(&checked["item"]),
                   "acceptance_criteria": ["builds", "lint is clean"]}),
        )
        .await;
    let ac = &edited["item"]["acceptance_criteria"];
    assert_eq!(ac[0]["checked"], true);
    assert_ne!(ac[1]["checked"], true);
    let raw = lead
        .call_raw(
            "item_update",
            json!({"id": id, "expected_version": version(&edited["item"]),
                                         "acceptance_criteria": ["  "]}),
        )
        .await;
    assert!(error_text(&raw).contains("blank"));
    let raw = lead
        .call_raw(
            "item_update",
            json!({"id": id, "expected_version": version(&edited["item"]),
                   "acceptance_criteria": ["builds", "builds"]}),
        )
        .await;
    assert!(error_text(&raw).contains("same text"));
    let ready = lead
        .call(
            "item_move",
            json!({"id": id, "to": "ready", "expected_version": version(&edited["item"])}),
        )
        .await;
    assert_eq!(ready["item"]["column_key"], "ready");
}
