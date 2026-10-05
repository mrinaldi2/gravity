//! A decision's option can grant permission extras (H-117): the owner's
//! ruling picking it is the grant, applied at publish with no second step.

mod common;

use bus::PermissionExtra;
use common::peers::{team, wait_until};
use common::tasks::project_with_bots;
use common::WsClient;
use serde_json::json;

/// A bot linked from another computer gets its extras there.
#[tokio::test]
async fn a_linked_bot_gets_its_grant_on_its_own_computer() {
    let mut t = team().await;
    let raised = t
        .lead
        .call(
            "raise_decision",
            json!({"title": "Let Windev install?", "body": "b",
                   "options": [{"key": "yes", "label": "Yes", "grants": [
                       {"bot": t.linked_windev, "extra": "install"}]}]}),
        )
        .await;
    let id = raised["decision"]["id"].as_str().expect("id").to_string();
    t.mac_client
        .request(json!({"type": "answer_decision", "decision_id": id,
                        "ruling_option": "yes", "ruling_text": "Yes."}))
        .await;
    t.mac_client
        .request(json!({"type": "publish_decisions", "items": [{"decision_id": id}]}))
        .await;
    let (win, windev) = (&t.win, t.windev_id.clone());
    wait_until("the PC grants it", || {
        win.app.db.bot_permission_extras(&windev).unwrap() == vec![PermissionExtra::Install]
    })
    .await;
}

#[tokio::test]
async fn the_owners_ruling_applies_the_extras_its_option_grants() {
    let (pair, mut bots) = project_with_bots(&["lead", "devops", "tester"]).await;
    let (devops, tester) = (pair.ids[1].clone(), pair.ids[2].clone());
    let raised = bots[0]
        .call(
            "raise_decision",
            json!({"title": "Let the bots install releases?", "body": "So you don't copy-paste.",
                   "options": [
                       {"key": "grant-all", "label": "Grant all", "grants": [
                           {"bot": "devops", "extra": "install"},
                           {"bot": "devops", "extra": "daemon_restart"},
                           {"bot": tester, "extra": "install"}]},
                       {"key": "hold", "label": "Hold"}]}),
        )
        .await;
    let id = raised["decision"]["id"].as_str().expect("id").to_string();
    let db = &pair.d.app.db;
    // Names are stored as ids, so a rename can't redirect a grant.
    let stored = db.get_decision(&id).unwrap().unwrap();
    assert_eq!(stored.options[0].grants[0].bot, devops);
    assert!(db.bot_permission_extras(&devops).unwrap().is_empty());

    let mut owner = WsClient::connect(&pair.d).await;
    owner
        .request(json!({"type": "answer_decision", "decision_id": id,
                        "ruling_option": "grant-all", "ruling_text": "Yes, all four."}))
        .await;
    // Drafted, not yet ruled: nothing granted.
    assert!(db.bot_permission_extras(&devops).unwrap().is_empty());
    let published = owner
        .request(json!({"type": "publish_decisions", "items": [{"decision_id": id}]}))
        .await;
    assert_eq!(published["type"], "publish_result", "{published}");

    assert_eq!(
        db.bot_permission_extras(&devops).unwrap(),
        vec![PermissionExtra::DaemonRestart, PermissionExtra::Install]
    );
    assert_eq!(
        db.bot_permission_extras(&tester).unwrap(),
        vec![PermissionExtra::Install]
    );
    let got = owner
        .request(json!({"type": "get_decision", "decision_id": id}))
        .await;
    assert!(
        got.to_string().contains("Applied this ruling's grants"),
        "{got}"
    );
}

#[tokio::test]
async fn a_grant_names_a_real_extra_and_a_bot_of_the_project() {
    let (_pair, mut bots) = project_with_bots(&["lead"]).await;
    for (grant, says) in [
        (json!({"bot": "lead", "extra": "root"}), "unknown extra"),
        (
            json!({"bot": "nobody", "extra": "install"}),
            "isn't a bot of this project",
        ),
    ] {
        let raw = bots[0]
            .call_raw(
                "raise_decision",
                json!({"title": format!("grant {says}"), "body": "b",
                       "options": [{"key": "yes", "label": "Yes", "grants": [grant]}]}),
            )
            .await;
        let text = common::tasks::error_text(&raw);
        assert!(text.contains(says), "{text}");
    }
}
