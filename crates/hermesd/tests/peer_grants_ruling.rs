//! A linked bot's grant stays bound to the ruling it came from (CE-020):
//! a ruling that no longer holds cancels it before it is sent (M1), a grant
//! decided before the owner changed the bot on its own computer is refused
//! there (M2), and one that can't be delivered within a day expires.

mod common;

use bus::PermissionExtra;
use chrono::{Duration, Utc};
use common::grants::{comments, desktop, drop_link, extras, grant, said, settled};
use common::peers::{team, Team};
use common::WsClient;
use hermesd::db::GrantRuling;
use serde_json::json;

/// The ruling's grants were waiting for the PC when this happened to it.
async fn waiting_then(t: &mut Team, change: impl AsyncFnOnce(&mut Team, &str)) -> String {
    let id = grant(t, &["install"], drop_link).await;
    said(&t.mac, &id, "Waiting for win").await;
    change(t, &id).await;
    id
}

/// (a) The lead raises a new decision that supersedes the ruling while the
/// PC is away: the grant is never sent.
#[tokio::test]
async fn a_superseded_ruling_grants_nothing_once_the_pc_is_back() {
    let mut t = team().await;
    let id = waiting_then(&mut t, async |t, id| {
        t.lead
            .call(
                "raise_decision",
                json!({"title": "Let Windev ship, take two", "body": "facts changed",
                       "supersedes": id}),
            )
            .await;
    })
    .await;
    said(&t.mac, &id, "No longer granted on win: the ruling changed").await;
    settled(&t).await;
    assert!(extras(&t).is_empty(), "nothing applied on the PC");
}

/// (b) The owner reopens the ruling while the PC is away: the same.
#[tokio::test]
async fn a_reopened_ruling_grants_nothing_once_the_pc_is_back() {
    let mut t = team().await;
    let id = waiting_then(&mut t, async |t, id| {
        let mut owner = desktop(&t.mac).await;
        let reopened = owner
            .request(json!({"type": "reopen_decision", "decision_id": id}))
            .await;
        assert_eq!(reopened["type"], "decision", "{reopened}");
    })
    .await;
    said(&t.mac, &id, "No longer granted on win: the ruling changed").await;
    settled(&t).await;
    assert!(extras(&t).is_empty(), "nothing applied on the PC");
}

/// (b) A grant bound to a decision that was withdrawn is cancelled, not
/// sent. A settled ruling can't be withdrawn in the app (it is reopened, as
/// above), so the row is filed here against a decision that was.
#[tokio::test]
async fn a_withdrawn_decision_grants_nothing() {
    let mut t = team().await;
    let raised = t
        .lead
        .call(
            "raise_decision",
            json!({"title": "Withdrawn", "body": "b",
                   "options": [{"key": "yes", "label": "Yes", "grants": [
                       {"bot": t.linked_windev, "extra": "install"}]}]}),
        )
        .await;
    let id = raised["decision"]["id"].as_str().expect("id").to_string();
    let stored = t.mac.app.db.get_decision(&id).unwrap().unwrap();
    let sha = hermesd::decisions::grants::sha(&stored.options[0]);
    let peer = t
        .mac
        .app
        .db
        .get_bot(&t.linked_windev)
        .unwrap()
        .unwrap()
        .peer_id
        .unwrap();
    let ruling = GrantRuling {
        decision_id: &id,
        grants_sha: &sha,
        decided_at: Utc::now(),
    };
    t.mac
        .app
        .db
        .insert_peer_grant(&ruling, &t.linked_windev, &peer, &["install".to_string()])
        .unwrap();
    let mut owner = desktop(&t.mac).await;
    let withdrawn = owner
        .request(json!({"type": "withdraw_decision", "decision_id": id, "reason": "no"}))
        .await;
    assert_eq!(withdrawn["decision"]["state"], "withdrawn", "{withdrawn}");
    hermesd::decisions::grants_peer::deliver(&t.mac.app, None).await;
    said(&t.mac, &id, "No longer granted on win: the ruling changed").await;
    assert!(extras(&t).is_empty());
}

/// (c) The owner changes Windev's extras on the PC after the ruling, while
/// the grant waits: the grant is refused there, not laid over the change.
#[tokio::test]
async fn a_change_made_on_the_pc_after_the_ruling_is_not_undone() {
    let mut t = team().await;
    let id = waiting_then(&mut t, async |t, _| {
        let mut pc_owner = WsClient::connect(&t.win).await;
        let set = pc_owner
            .request(
                json!({"type": "set_bot_permission_extras", "bot_id": t.windev_id,
                            "extras": ["quiesce"]}),
            )
            .await;
        assert_eq!(set["type"], "bot", "{set}");
    })
    .await;
    said(
        &t.mac,
        &id,
        "Not granted on win: windev's install (the owner changed its extras here after the \
         ruling; set it here); set it on win",
    )
    .await;
    settled(&t).await;
    assert_eq!(extras(&t), vec![PermissionExtra::Quiesce]);
}

/// (d) A change on the PC from before the ruling doesn't stand in its way.
#[tokio::test]
async fn a_change_made_before_the_ruling_lets_it_apply() {
    let mut t = team().await;
    let mut pc_owner = WsClient::connect(&t.win).await;
    pc_owner
        .request(
            json!({"type": "set_bot_permission_extras", "bot_id": t.windev_id,
                        "extras": ["quiesce"]}),
        )
        .await;
    let id = grant(&mut t, &["install"], |_| {}).await;
    said(&t.mac, &id, "Granted on win: windev now has install.").await;
    assert_eq!(
        extras(&t),
        vec![PermissionExtra::Install, PermissionExtra::Quiesce]
    );
}

/// A grant still waiting a day after its ruling isn't sent: it expires,
/// and the decision says to set it on the PC.
#[tokio::test]
async fn a_grant_waiting_a_day_expires() {
    let mut t = team().await;
    // A settled ruling whose grants this computer filed earlier.
    let id = grant(&mut t, &["publish"], |_| {}).await;
    let stored = t.mac.app.db.get_decision(&id).unwrap().unwrap();
    let sha = hermesd::decisions::grants::sha(&stored.options[0]);
    let peer = t
        .mac
        .app
        .db
        .get_bot(&t.linked_windev)
        .unwrap()
        .unwrap()
        .peer_id
        .unwrap();
    let ruling = GrantRuling {
        decision_id: &id,
        grants_sha: &sha,
        decided_at: Utc::now(),
    };
    t.mac
        .app
        .db
        .insert_peer_grant(&ruling, &t.linked_windev, &peer, &["install".to_string()])
        .unwrap();
    let conn = rusqlite::Connection::open(t.mac.app.cfg.db_path()).unwrap();
    let old = (Utc::now() - Duration::hours(25)).to_rfc3339();
    conn.execute("UPDATE peer_grant SET created_at = ?1", [old])
        .unwrap();
    hermesd::decisions::grants_peer::deliver(&t.mac.app, None).await;
    said(&t.mac, &id, "Not sent: win was unreachable for a day").await;
    assert!(comments(&t.mac, &id)
        .iter()
        .any(|c| c.contains("set it on win")));
    assert!(extras(&t).is_empty());
}
