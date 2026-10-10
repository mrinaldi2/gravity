//! Stacked cards in one release (H-287): a card blocked by another is
//! ready once its blocker's work is reviewed and in Verify, not only once it
//! is Done, which a feature reaches only when its release ships.

mod common;

use common::releases::releases;
use common::tasks::error_text;
use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority, Size};
use hermesd::db::{MoveTo, NewItem};
use serde_json::json;

#[tokio::test]
async fn a_card_stacked_on_one_in_verify_goes_to_ready() {
    let mut r = releases(0).await;
    let db = r.pair.d.app.db.clone();
    let card = |title: &str| {
        db.create_item(
            &NewItem {
                project_id: &r.project,
                item_type: ItemType::Feature,
                title,
                description: "",
                platforms: &[Platform::Daemon],
                size: Some(Size::S),
                priority: Priority::P1,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &["it works".to_string()],
            },
            &Actor::User,
        )
        .unwrap()
        .id
    };
    let (a, b) = (card("Contract"), card("Core on the contract"));
    let lead = &mut r.bots[0];
    lead.call("release_plan", json!({"name": "0.18.0", "items": [a, b]}))
        .await;
    lead.call(
        "item_link",
        json!({"id": a, "kind": "item:blocks", "ref": b}),
    )
    .await;
    lead.call(
        "item_link",
        json!({"id": b, "kind": "artifact", "ref": "spec.md", "label": "spec"}),
    )
    .await;
    let ready = |version: u64| json!({"id": b, "to": "ready", "expected_version": version});
    let version = |id: &str| db.get_item(id).unwrap().unwrap().version;

    // A still in Doing: B waits for it.
    let to = |column| MoveTo {
        column,
        ..MoveTo::default()
    };
    db.move_item(&a, version(&a), &to("doing"), &Actor::User)
        .unwrap();
    let raw = r.bots[0].call_raw("item_move", ready(version(&b))).await;
    let why = error_text(&raw);
    assert!(
        why.contains("dor.blocked_by") || why.contains("blocks it"),
        "{why}"
    );

    // A reviewed and in Verify, its release not shipped: B goes to Ready.
    db.move_item(&a, version(&a), &to("verify"), &Actor::User)
        .unwrap();
    let moved = r.bots[0].call("item_move", ready(version(&b))).await;
    assert_eq!(moved["item"]["column_key"], "ready", "{moved}");
}
