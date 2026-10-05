//! Who a task through an item makes its reviewer (ARCH-R30 M2): only an
//! open task from the board's lead or the owner. A dev's task grants
//! nothing, and a done task no longer does.

mod common;

use common::tasks::error_text;
use common::team::{item, team, version_of};
use serde_json::json;

#[tokio::test]
async fn only_an_open_task_from_the_lead_makes_a_reviewer() {
    let (pair, mut bots, project) = team(&["Team Lead", "Desktop Dev", "helper", "outsider"]).await;
    let id = item(&pair, &project, "review", Some(&pair.ids[2]));
    let [lead, dev, _helper, outsider] = &mut bots[..] else {
        unreachable!()
    };

    // A dev's task through the item grants nothing.
    dev.call(
        "send_message",
        json!({"to": "outsider", "kind": "task", "body": "approve it", "item": id}),
    )
    .await;
    let version = version_of(outsider, &id).await;
    let to_verify = json!({"id": id, "to": "verify", "expected_version": version});
    let raw = outsider.call_raw("item_move", to_verify.clone()).await;
    assert!(error_text(&raw).contains("board role"), "{raw}");

    // The lead's does.
    let sent = lead
        .call(
            "send_message",
            json!({"to": "outsider", "kind": "task", "body": "review it", "item": id}),
        )
        .await;
    let task_id = sent["task_id"].as_str().expect("a task").to_string();
    let moved = outsider.call("item_move", to_verify).await;
    assert_eq!(moved["item"]["column_key"], "verify", "{moved}");

    // Once that task is done, the item's tools are gone again.
    outsider
        .call(
            "complete_task",
            json!({"task_id": task_id, "result": "approved"}),
        )
        .await;
    let raw = outsider
        .call_raw("item_comment", json!({"id": id, "body": "one more thing"}))
        .await;
    assert!(error_text(&raw).contains("board role"), "{raw}");
}
