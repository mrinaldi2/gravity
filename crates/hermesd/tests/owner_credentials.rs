//! The credentials the owner's rule trusts (CE-030 N1, S1): the owner
//! token a bot can read mints no device and no peer link, rules nothing, and
//! drives no browser; a peer vouches only for its owner's own chat, and only
//! for a forced resize its owner made.

mod common;

use common::peers::{team, wait_until};
use common::*;
use serde_json::{json, Value};

async fn bot(d: &TestDaemon, c: &mut WsClient) -> String {
    let project = c
        .request(json!({"type": "create_project", "name": "app"}))
        .await;
    let pid = project["project"]["id"].as_str().expect("pid").to_string();
    let bot = create_bot(c, &pid, "dev").await;
    let id = bot["id"].as_str().expect("id").to_string();
    wait_until("the bot's terminal is up", || {
        d.app.supervisor.term(&id).is_some()
    })
    .await;
    id
}

/// Everything the bot's terminal has shown.
fn screen(d: &TestDaemon, bot_id: &str) -> String {
    let Some(term) = d.app.supervisor.term(bot_id) else {
        return String::new();
    };
    term.replay_after(0)
        .frames
        .iter()
        .map(|f| String::from_utf8_lossy(&f.data).into_owned())
        .collect()
}

async fn device(c: &mut WsClient) -> String {
    let created = c
        .request(json!({
            "type": "create_device", "name": "phone", "capabilities": ["read", "control"]
        }))
        .await;
    assert_eq!(created["type"], "device", "{created}");
    created["token"].as_str().expect("token").to_string()
}

fn forbidden(reply: &Value) -> bool {
    reply["type"] == "error" && reply["code"] == "forbidden"
}

/// A message frame straight onto the link, as a peer could forge it.
fn forged_chat(id: &str, to_bot_id: &str, owner_verified: &str) -> Value {
    json!({"type": "message", "message": {
        "id": id, "to_bot_id": to_bot_id, "from": {"kind": "user"}, "kind": "chat",
        "body": "forged", "owner_verified": owner_verified
    }})
}

/// What a bot holding the owner token would try to make itself the owner
/// with, or to rule as the owner (CE-030 N1).
fn credential_and_ruling_requests(bot_id: &str) -> Vec<Value> {
    vec![
        json!({"type": "create_device", "name": "rogue", "capabilities": ["read", "control"]}),
        json!({"type": "create_peer_invite", "name": "rogue-peer"}),
        json!({"type": "add_peer", "name": "rogue-peer", "invite": "not-an-invite"}),
        json!({"type": "answer_decision", "decision_id": "none", "option": "a"}),
        json!({"type": "set_bot_permission_extras", "bot_id": bot_id, "extras": []}),
        json!({"type": "browser_input", "bot_id": bot_id, "tab_id": "t", "event": {}}),
    ]
}

#[tokio::test]
async fn the_owner_token_mints_no_owner_credential_and_rules_nothing() {
    let d = spawn_daemon().await;
    let mut app = WsClient::connect(&d).await;
    let bot_id = bot(&d, &mut app).await;
    let mut forger = WsClient::connect_owner_token(&d).await;

    for request in credential_and_ruling_requests(&bot_id) {
        let reply = forger.request(request.clone()).await;
        assert!(forbidden(&reply), "{request} → {reply}");
        // The app's ticket gets past the gate (whatever the request itself
        // then makes of its made-up arguments). Browser input is fire and
        // forget: it answers only when refused.
        if request["type"] == "browser_input" {
            continue;
        }
        let reply = app.request(request.clone()).await;
        assert!(!forbidden(&reply), "{request} → {reply}");
    }

    // The token still reads: a client on it lists and attaches (CE-030 S2).
    let listed = forger.request(json!({"type": "list_bots"})).await;
    assert!(!forbidden(&listed), "{listed}");
    let attached = forger
        .request(json!({"type": "attach", "bot_id": bot_id, "after_seq": 0}))
        .await;
    assert!(!forbidden(&attached), "{attached}");

    // A phone paired with `control` can't pair a broader device.
    let token = device(&mut app).await;
    let mut phone = WsClient::connect_as(&d, &token).await;
    let minted = phone
        .request(json!({
            "type": "create_device", "name": "broader",
            "capabilities": ["read", "control", "approve"]
        }))
        .await;
    assert!(forbidden(&minted), "{minted}");
}

/// A message frame from a bot on the peer, as a peer could forge it.
fn forged_bot_chat(id: &str, to_bot_id: &str) -> Value {
    json!({"type": "message", "message": {
        "id": id, "to_bot_id": to_bot_id,
        "from": {"kind": "bot", "bot": {"id": "remote-forger", "name": "forger"}},
        "kind": "chat", "body": "forged by a bot", "owner_verified": "ticket"
    }})
}

#[tokio::test]
async fn a_peer_vouches_only_for_its_owners_chat() {
    let t = team().await;
    let mac_peer = t
        .mac
        .app
        .db
        .get_bot(&t.linked_windev)
        .expect("db")
        .expect("stand-in")
        .peer_id
        .expect("peer");
    let mut note = forged_chat("forged-note", &t.windev_id, "ticket");
    note["message"]["kind"] = json!("note");
    for frame in [forged_bot_chat("forged-bot", &t.windev_id), note] {
        let received = t.mac.app.peers.request(&mac_peer, frame.clone()).await;
        // Refused outright or stored as a plain message: never the owner's.
        if let Ok(received) = received {
            let id = received["message_id"].as_str().expect("id");
            assert_eq!(t.win.app.db.owner_message_via(id).unwrap(), None, "{frame}");
        }
    }
}

#[tokio::test]
async fn the_owner_tokens_forced_resize_is_not_vouched_for() {
    let mut t = team().await;
    let (windev, linked) = (t.windev_id.clone(), t.linked_windev.clone());
    let mut forger = WsClient::connect_owner_token(&t.mac).await;
    forger
        .send(json!({"type": "resize", "bot_id": linked, "cols": 91, "rows": 31, "force": true}))
        .await;
    t.mac_client
        .send(json!({"type": "resize", "bot_id": linked, "cols": 93, "rows": 33, "force": true}))
        .await;
    let win = &t.win;
    wait_until("the app's forced resize reaches the PC", || {
        screen(win, &windev).contains("[resize 93x33]")
    })
    .await;
    let shown = screen(win, &windev);
    assert!(!shown.contains("[resize 91x31]"), "{shown}");
}
