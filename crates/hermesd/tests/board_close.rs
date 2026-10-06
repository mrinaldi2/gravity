//! Spikes and chores without code close on their outcome (H-154): the lead
//! or a reviewer other than the assignee moves them Verify → Done once an
//! outcome artifact is linked, and the history names it. Features and bugs
//! keep the release path, and a spike not yet started is told how to close.

mod common;

use common::tasks::error_text;
use common::team::{get, item, team, version_of};
use serde_json::json;

#[tokio::test]
async fn a_spike_closes_on_its_outcome_by_the_lead_not_its_assignee() {
    let (pair, mut bots, project) = team(&["Team Lead", "Desktop Dev"]).await;
    let dev = pair.ids[1].clone();
    let id = item(&pair, &project, "verify", Some(&dev));
    let [lead, worker] = &mut bots[..] else {
        unreachable!()
    };
    let version = version_of(lead, &id).await;
    lead.call(
        "item_update",
        json!({"id": id, "expected_version": version, "type": "spike"}),
    )
    .await;

    // Without an outcome, nobody closes it.
    let version = version_of(lead, &id).await;
    let close = |version: &str| json!({"id": id, "to": "done", "expected_version": version});
    let raw = lead.call_raw("item_move", close(&version)).await;
    assert!(error_text(&raw).contains("No outcome artifact"), "{raw}");

    worker
        .call(
            "item_link",
            json!({"id": id, "kind": "artifact", "ref": "H-x-answer.md", "label": "outcome"}),
        )
        .await;
    // The assignee doesn't close their own spike.
    let version = version_of(worker, &id).await;
    let raw = worker.call_raw("item_move", close(&version)).await;
    assert!(
        error_text(&raw).contains("the lead, or a reviewer"),
        "{raw}"
    );

    // The lead does, and the history says what it closed on.
    let moved = lead.call("item_move", close(&version)).await;
    assert_eq!(moved["item"]["column_key"], "done", "{moved}");
    let full = lead.call("item_get", json!({ "id": id })).await;
    let last = full["history"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|h| h["kind"] == "moved")
        .cloned()
        .unwrap();
    assert_eq!(last["to"], "done");
    assert_eq!(
        last["note"], "closed on its outcome: H-x-answer.md",
        "{last}"
    );
}

#[tokio::test]
async fn features_keep_the_release_path_and_unstarted_spikes_are_told_how() {
    let (pair, mut bots, project) = team(&["Team Lead", "Desktop Dev"]).await;
    let feature = item(&pair, &project, "verify", Some(&pair.ids[1]));
    let spike = item(&pair, &project, "inbox", None);
    let lead = &mut bots[0];
    lead.call(
        "item_link",
        json!({"id": feature, "kind": "artifact", "ref": "notes.md"}),
    )
    .await;
    let version = version_of(lead, &feature).await;
    let raw = lead
        .call_raw(
            "item_move",
            json!({"id": feature, "to": "done", "expected_version": version}),
        )
        .await;
    assert!(error_text(&raw).contains("Only spikes and chores"), "{raw}");

    let version = get(lead, &spike).await["version"]
        .as_str()
        .unwrap()
        .to_string();
    lead.call(
        "item_update",
        json!({"id": spike, "expected_version": version, "type": "spike"}),
    )
    .await;
    let version = version_of(lead, &spike).await;
    let raw = lead
        .call_raw(
            "item_move",
            json!({"id": spike, "to": "done", "expected_version": version}),
        )
        .await;
    assert!(
        error_text(&raw).contains("from Doing, Review or Verify"),
        "{raw}"
    );
}
