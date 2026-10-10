//! A decision's option can grant permission extras (H-117): the owner's
//! ruling picking it is the grant, applied at publish with no second step.
//! Only a client that shows grants rules on one, pinning the grants it
//! showed by their sha (ARCH-R51 M2).

mod common;

use bus::PermissionExtra;
use common::peers::{team_trusting, wait_until};
use common::tasks::project_with_bots;
use common::{TestDaemon, WsClient};
use serde_json::{json, Value};

/// The owner's desktop: it says it shows what an option grants.
async fn desktop(d: &TestDaemon) -> WsClient {
    WsClient::connect_with(d, &d.app.owner.mint(), &["decision_grants"]).await
}

/// The `grants_sha` of option `key` as the client was shown it.
async fn shown_sha(owner: &mut WsClient, id: &str, key: &str) -> Value {
    let got = owner
        .request(json!({"type": "get_decision", "decision_id": id}))
        .await;
    let options = got["decision"]["options"].as_array().expect("options");
    let option = options.iter().find(|o| o["key"] == key).expect("option");
    option["grants_sha"].clone()
}

/// A bot linked from another computer gets its extras there.
#[tokio::test]
async fn a_linked_bot_gets_its_grant_on_its_own_computer() {
    let mut t = team_trusting().await;
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
    let mut owner = desktop(&t.mac).await;
    let sha = shown_sha(&mut owner, &id, "yes").await;
    owner
        .request(
            json!({"type": "publish_decisions", "items": [{"decision_id": id,
                        "ruling_option": "yes", "ruling_text": "Yes.", "grants_sha": sha}]}),
        )
        .await;
    let (win, windev) = (&t.win, t.windev_id.clone());
    wait_until("the PC grants it", || {
        win.app.db.bot_permission_extras(&windev).unwrap() == vec![PermissionExtra::Install]
    })
    .await;
    // The PC's owner sees who granted what, from where (ARCH-R51 S1b).
    let mut pc_owner = WsClient::connect(win).await;
    let grants = pc_owner
        .request(json!({"type": "bot_grants", "bot_id": windev}))
        .await;
    assert_eq!(grants["grants"][0]["from"], "mac", "{grants}");
    assert_eq!(grants["grants"][0]["extras"], json!(["install"]));
    assert_eq!(grants["grants"][0]["decision"], "Let Windev install?");
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
    assert!(stored.options[0].grants_sha.is_none(), "never stored");
    assert!(db.bot_permission_extras(&devops).unwrap().is_empty());

    let answer = |sha: Value| {
        json!({"type": "answer_decision", "decision_id": id, "ruling_option": "grant-all",
               "ruling_text": "Yes, all four.", "grants_sha": sha})
    };
    // A client that can't show grants may not pick it; the hold option is fine.
    let mut phone = WsClient::connect(&pair.d).await;
    let refused = phone.request(answer(Value::Null)).await;
    assert_eq!(refused["code"], "forbidden", "{refused}");
    assert!(refused["message"]
        .as_str()
        .unwrap()
        .contains("Answer this on the desktop"));

    let mut owner = desktop(&pair.d).await;
    let sha = shown_sha(&mut owner, &id, "grant-all").await;
    assert!(sha.is_string(), "{sha}");
    // Grants other than the ones shown: refused.
    let stale = owner.request(answer(json!("0".repeat(64)))).await;
    assert_eq!(stale["code"], "conflict", "{stale}");
    owner.request(answer(sha.clone())).await;
    // Drafted, not yet ruled: nothing granted.
    assert!(db.bot_permission_extras(&devops).unwrap().is_empty());
    // Publishing the draft from a client that can't show it: refused too.
    let from_phone = phone
        .request(json!({"type": "publish_decisions", "items": [{"decision_id": id}]}))
        .await;
    assert_eq!(from_phone["code"], "forbidden", "{from_phone}");
    let published = owner
        .request(json!({"type": "publish_decisions",
                        "items": [{"decision_id": id, "grants_sha": sha}]}))
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
    // Hermes applied them, not the owner (H-173).
    let applied = got["decision"]["comments"]
        .as_array()
        .and_then(|cs| {
            cs.iter().find(|c| {
                c["body"]
                    .as_str()
                    .is_some_and(|b| b.starts_with("Applied this ruling's grants"))
            })
        })
        .unwrap_or_else(|| panic!("no applied-grants comment: {got}"));
    assert_eq!(applied["author_kind"], "system", "{applied}");
    assert_eq!(applied["author_name"], "Hermes", "{applied}");
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
