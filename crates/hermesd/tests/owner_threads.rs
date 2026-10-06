//! Owner threads and pins on one computer (H-128 D6, D7): a bot's
//! `message_owner` shows in its thread, `asks` and `item_comment
//! asks_owner` are owner_question rows until the owner answers or dismisses
//! them, unread counts follow `owner_thread_read`, and a pin ranks first.

mod common;

use bus::contract::home::{
    home_request::Request, home_response::Response, HomeRequest, OwnerThreadGetRequest,
};
use bus::contract::wire::{envelope::Body, Envelope};
use common::board::{new_item, next_envelope, send_raw};
use common::peers::project;
use common::tasks::project_with_bots;
use common::*;
use hermesd::board::model::{Priority, ProjectRole, Role};
use prost::Message;
use serde_json::{json, Value};
use std::time::Duration;

async fn rows(owner: &mut WsClient, project_id: &str, kind: &str) -> Vec<Value> {
    let reply = owner
        .request(json!({"type": "attention_rows", "project_id": project_id}))
        .await;
    reply["attention_rows"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r["kind"] == kind)
        .collect()
}

async fn threads(owner: &mut WsClient) -> Value {
    let reply = owner.request(json!({"type": "owner_threads"})).await;
    assert_eq!(reply["type"], "owner_threads", "{reply}");
    reply["owner_threads"]["threads"].clone()
}

fn thread_of<'a>(threads: &'a Value, bot_id: &str) -> &'a Value {
    threads
        .as_array()
        .expect("threads")
        .iter()
        .find(|t| t["bot"]["bot_id"] == bot_id)
        .unwrap_or_else(|| panic!("no thread for {bot_id}: {threads}"))
}

#[tokio::test]
async fn a_bots_message_shows_in_its_thread_and_a_question_waits_for_the_owner() {
    let (pair, mut bots) = project_with_bots(&["Lead", "Dev"]).await;
    let lead = pair.ids[0].clone();
    let project_id = pair.d.app.db.get_bot(&lead).unwrap().unwrap().project_id;
    let mut owner = WsClient::connect(&pair.d).await;
    owner
        .request(json!({"type": "send_user_message", "to_bot_id": lead, "body": "How is 0.17?"}))
        .await;
    let sent = bots[0]
        .call(
            "message_owner",
            json!({"body": "Should 0.17 wait for iOS?", "asks": true}),
        )
        .await;
    assert_eq!(sent["asks"], true, "{sent}");
    let pushed = owner
        .wait_for(|v| v["type"] == "owner_thread_updated")
        .await;
    assert_eq!(pushed["bot"]["bot_id"], lead.as_str());
    assert_eq!(pushed["project_id"], project_id.as_str());
    assert!(pushed.get("title").is_none(), "no row text in pushes (F1)");

    // (a) The thread holds both sides; the entry counts it unread.
    let page = owner
        .request(json!({"type": "owner_thread_get", "bot_id": lead}))
        .await;
    assert_eq!(page["type"], "owner_thread", "{page}");
    let messages = page["owner_thread"]["messages"]
        .as_array()
        .expect("messages");
    assert_eq!(messages.len(), 2, "{page}");
    assert_eq!(messages[0]["from_owner"], true);
    assert_eq!(messages[1]["text"], "Should 0.17 wait for iOS?");
    assert_eq!(messages[1]["asks"], true);
    assert_eq!(messages[1]["open"], true);
    let all = threads(&mut owner).await;
    assert_eq!(
        all[0]["bot"]["bot_id"],
        lead.as_str(),
        "open questions first"
    );
    let entry = thread_of(&all, &lead);
    assert_eq!(entry["unread"], 1);
    assert_eq!(entry["open_question"], true);
    assert_eq!(entry["last"]["asks"], true);
    assert!(thread_of(&all, &pair.ids[1]).get("last").is_none());

    // (b) One owner_question row, aimed at the thread; it goes when the
    // owner writes in it.
    let asked = rows(&mut owner, &project_id, "OWNER_QUESTION").await;
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0]["bot"]["bot_id"], lead.as_str());
    assert_eq!(asked[0]["title"], "Lead asks: Should 0.17 wait for iOS?");
    owner
        .request(json!({"type": "send_user_message", "to_bot_id": lead, "body": "Wait."}))
        .await;
    assert!(rows(&mut owner, &project_id, "OWNER_QUESTION")
        .await
        .is_empty());
    // proto3 JSON leaves false out.
    assert!(thread_of(&threads(&mut owner).await, &lead)
        .get("open_question")
        .is_none());

    // (c) Unread follows owner_thread_read.
    let num = messages[1]["num"].as_str().expect("int64 as a string");
    let marked = owner
        .request(json!({"type": "owner_thread_read", "bot_id": lead, "up_to_num": num}))
        .await;
    assert_eq!(marked["type"], "owner_thread_marked", "{marked}");
    assert!(
        marked["owner_thread_marked"].get("unread").is_none(),
        "0: {marked}"
    );
    assert!(thread_of(&threads(&mut owner).await, &lead)
        .get("unread")
        .is_none());

    // Too long is refused; nothing is stored.
    let long = bots[0]
        .call_raw("message_owner", json!({"body": "x".repeat(4097)}))
        .await;
    assert_eq!(long["isError"], true, "{long}");
}

#[tokio::test]
async fn a_question_on_a_card_waits_for_the_owners_comment_or_a_dismissal() {
    let (pair, mut bots) = project_with_bots(&["Lead"]).await;
    let lead = pair.ids[0].clone();
    let db = &pair.d.app.db;
    let project_id = db.get_bot(&lead).unwrap().unwrap().project_id;
    db.ensure_board(&project_id, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    db.set_project_role(&ProjectRole {
        project_id: project_id.clone(),
        role: Role::Lead,
        bot_id: lead.clone(),
        machine: None,
    })
    .unwrap();
    let (card, _) = new_item(db, &project_id, "Owner threads", Priority::P1);
    let tools = bots[0].tools().await;
    let comment_tool = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "item_comment")
        .expect("item_comment");
    assert!(comment_tool["inputSchema"]["properties"]["asks_owner"].is_object());
    assert!(tools.to_string().contains("\"message_owner\""));

    let asked = bots[0]
        .call(
            "item_comment",
            json!({"id": card, "body": "Ship without the iPad layout?", "asks_owner": true}),
        )
        .await;
    assert!(asked["owner_question"].is_string(), "{asked}");
    let mut owner = WsClient::connect(&pair.d).await;
    let row = rows(&mut owner, &project_id, "OWNER_QUESTION").await;
    assert_eq!(row.len(), 1);
    assert_eq!(row[0]["item_id"], card.as_str());
    // The owner's card message is an owner comment there (D5): it answers.
    owner
        .request(json!({"type": "send_user_message", "to_bot_id": lead,
                        "body": "Yes, ship it.", "item_id": card}))
        .await;
    assert!(rows(&mut owner, &project_id, "OWNER_QUESTION")
        .await
        .is_empty());

    // A thread question, dismissed by the owner (approve).
    bots[0]
        .call(
            "message_owner",
            json!({"body": "Rename the app?", "asks": true, "item": card}),
        )
        .await;
    let row = rows(&mut owner, &project_id, "OWNER_QUESTION").await;
    assert_eq!(row.len(), 1);
    let comments = db.board_read(|t| t.item_comments(&card)).unwrap();
    assert!(comments
        .iter()
        .any(|c| c.body == "To the owner: Rename the app?"));
    let dismissed = owner
        .request(json!({"type": "attention_dismiss", "id": row[0]["id"]}))
        .await;
    assert_eq!(dismissed["type"], "attention_dismissed", "{dismissed}");
    assert!(rows(&mut owner, &project_id, "OWNER_QUESTION")
        .await
        .is_empty());
    let again = owner
        .request(json!({"type": "attention_dismiss", "id": row[0]["id"]}))
        .await;
    assert_eq!(again["code"], "not_found", "{again}");
}

#[tokio::test]
async fn a_pinned_project_ranks_first_and_pinning_is_controls() {
    let (pair, mut bots) = project_with_bots(&["Lead"]).await;
    let busy = pair
        .d
        .app
        .db
        .get_bot(&pair.ids[0])
        .unwrap()
        .unwrap()
        .project_id;
    bots[0]
        .call(
            "raise_decision",
            json!({"title": "Budget", "body": "Which plan?"}),
        )
        .await;
    let mut owner = WsClient::connect(&pair.d).await;
    let calm = project(&mut owner, "calm").await;
    let first = |o: &Value| o["rows"][0]["project_id"].as_str().unwrap().to_string();
    let o = owner.request(json!({"type": "projects_overview"})).await;
    assert_eq!(first(&o["overview"]), busy);

    let pinned = owner
        .request(json!({"type": "project_pin", "project_id": calm, "pinned": true}))
        .await;
    assert_eq!(pinned["type"], "project_pinned", "{pinned}");
    assert_eq!(pinned["pinned"], true);
    let o = owner.request(json!({"type": "projects_overview"})).await;
    assert_eq!(first(&o["overview"]), calm);
    assert_eq!(o["overview"]["rows"][0]["pinned"], true);

    let created = owner
        .request(json!({"type": "create_device", "name": "phone", "capabilities": ["read"]}))
        .await;
    let mut reader = WsClient::connect_as(&pair.d, token_str(&created)).await;
    let refused = reader
        .request(json!({"type": "project_pin", "project_id": calm, "pinned": false}))
        .await;
    assert_eq!(refused["code"], "forbidden", "{refused}");
    // Threads are read's.
    let listed = reader.request(json!({"type": "owner_threads"})).await;
    assert_eq!(listed["type"], "owner_threads", "{listed}");
}

#[tokio::test]
async fn a_thread_reads_the_same_over_binary_frames() {
    let (pair, mut bots) = project_with_bots(&["Lead"]).await;
    let lead = pair.ids[0].clone();
    bots[0]
        .call("message_owner", json!({"body": "Heads-up: CI is red."}))
        .await;
    let mut owner = WsClient::connect(&pair.d).await;
    let json_page = owner
        .request(json!({"type": "owner_thread_get", "bot_id": lead}))
        .await;
    let request = Envelope {
        req_id: 9,
        body: Some(Body::HomeRequest(HomeRequest {
            request: Some(Request::OwnerThreadGet(OwnerThreadGetRequest {
                bot_id: lead.clone(),
                ..OwnerThreadGetRequest::default()
            })),
        })),
    };
    send_raw(&mut owner, request.encode_to_vec()).await;
    let envelope = next_envelope(&mut owner, Duration::from_secs(5))
        .await
        .expect("an answer");
    assert_eq!(envelope.req_id, 9);
    let Some(Body::HomeResponse(response)) = envelope.body else {
        panic!("expected a home response");
    };
    let Some(Response::OwnerThread(page)) = response.response else {
        panic!("expected the thread");
    };
    let decoded: bus::contract::home::OwnerThreadPage =
        serde_json::from_value(json_page["owner_thread"].clone()).expect("proto3 JSON");
    assert_eq!(page, decoded);
    assert_eq!(page.messages[0].text, "Heads-up: CI is red.");
}
