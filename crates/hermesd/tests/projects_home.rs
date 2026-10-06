//! The projects home on one computer (H-128 D1, D2, D4): `projects_overview`
//! with one ranked row per project, the attention it counts being exactly
//! `attention_rows`, the same message over JSON and binary frames, grants,
//! and `projects_overview_changed` pushed debounced.

mod common;

use std::time::{Duration, Instant};

use bus::contract::home::{
    home_request::Request, home_response::Response, HomeRequest, ProjectsOverview,
    ProjectsOverviewRequest,
};
use bus::contract::wire::{envelope::Body, Envelope};
use bus::BotState;
use common::board::{new_item, next_envelope, send_raw, walk};
use common::peers::{project, wait_until};
use common::tasks::project_with_bots;
use common::*;
use hermesd::board::model::Priority;
use prost::Message;
use serde_json::{json, Value};

async fn overview(c: &mut WsClient) -> Value {
    let reply = c.request(json!({"type": "projects_overview"})).await;
    assert_eq!(reply["type"], "projects_overview", "{reply}");
    reply["overview"].clone()
}

fn row<'a>(o: &'a Value, project_id: &str) -> &'a Value {
    o["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["project_id"] == project_id)
        .unwrap_or_else(|| panic!("no row for {project_id}: {o}"))
}

#[tokio::test]
async fn each_project_has_a_row_counting_what_its_needs_you_lists() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "Desktop Dev"]).await;
    let app = &pair.d.app;
    let db = &app.db;
    let busy = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    let me = db.daemon_id().unwrap();
    db.ensure_board(&busy, &me, Some("H")).unwrap();
    let (urgent, _) = new_item(db, &busy, "Crash on start", Priority::P0);
    bots[0]
        .call(
            "raise_decision",
            json!({"title": "Budget for push", "body": "Which plan?"}),
        )
        .await;
    // A bot waiting for the owner is a row too (D1 b).
    app.supervisor
        .set_state(&pair.ids[1], BotState::WaitingForUser, "Didn't connect");
    let mut owner = WsClient::connect(&pair.d).await;
    let calm = project(&mut owner, "calm").await;

    let o = overview(&mut owner).await;
    let busy_row = row(&o, &busy);
    let attention = &busy_row["attention"];
    assert_eq!(attention["by_kind"]["p0_item"], 1, "{busy_row}");
    assert_eq!(attention["by_kind"]["decision"], 1, "{busy_row}");
    assert_eq!(attention["by_kind"]["bot_waiting"], 1, "{busy_row}");
    assert_eq!(attention["count"], 3);
    assert_eq!(attention["score"], 2 + 1 + 1);
    assert_eq!(busy_row["board_home"], me);
    assert_eq!(busy_row["bots"], 2);
    assert_eq!(busy_row["members"][0]["daemon_id"], me);
    assert_eq!(busy_row["rank"], Value::Null, "rank 0 is proto3's default");
    // The card's extras (H-144): who waits for the owner, the board's counts.
    assert_eq!(busy_row["bots_waiting"], 1, "{busy_row}");
    assert!(busy_row.get("doing_total").is_none(), "nothing in Doing");
    let inbox = &busy_row["columns"][0];
    assert_eq!(inbox["category"], "inbox", "{busy_row}");
    assert_eq!(inbox["count"], 1, "{busy_row}");
    // A project without a board has no home and nothing in Doing (D2 d).
    let calm_row = row(&o, &calm);
    assert!(calm_row.get("board_home").is_none(), "{calm_row}");
    assert!(calm_row.get("doing").is_none(), "{calm_row}");
    assert!(calm_row.get("columns").is_none(), "{calm_row}");
    assert_eq!(calm_row["rank"], 1, "the busy project ranks first (D2 b)");
    assert_eq!(o["total"]["count"], 3);
    assert!(o["as_of"].is_string());

    // The overview's count is the project's Needs-you list (D1 c).
    let reply = owner
        .request(json!({"type": "attention_rows", "project_id": busy}))
        .await;
    let rows = reply["attention_rows"]["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 3, "{reply}");
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids[0], format!("p0_item:{me}:{urgent}"), "weight first");
    // Ids are stable across reads (D1 e), and each row says where to act
    // on it (D1 f).
    let again = owner
        .request(json!({"type": "attention_rows", "project_id": busy}))
        .await;
    assert_eq!(
        again["attention_rows"]["rows"],
        reply["attention_rows"]["rows"]
    );
    for r in rows {
        assert_eq!(r["daemon_id"], me);
        let target = match r["kind"].as_str().unwrap() {
            "P0_ITEM" => "item_id",
            "DECISION" => "decision_id",
            "BOT_WAITING" => "bot",
            other => panic!("unexpected kind {other}"),
        };
        assert!(r.get(target).is_some(), "{r}");
    }

    // The dashboard lists the same rows; the newer kinds only when asked.
    let dashboard =
        |all: bool| json!({"type": "dashboard_get", "project_id": busy, "all_kinds": all});
    let old = owner.request(dashboard(false)).await;
    let kinds: Vec<&str> = old["dashboard"]["needs_you"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["decision", "p0"]);
    let all = owner.request(dashboard(true)).await;
    assert_eq!(all["dashboard"]["needs_you"].as_array().unwrap().len(), 3);
}

/// Doing lists three cards and counts them all; each column counts its
/// cards, in board order (H-144).
#[tokio::test]
async fn a_row_counts_every_column_and_all_of_doing() {
    let (pair, _bots) = project_with_bots(&["Lead"]).await;
    let db = &pair.d.app.db;
    let project_id = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.ensure_board(&project_id, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    for n in 0..4 {
        let (id, _) = new_item(db, &project_id, &format!("Doing {n}"), Priority::P2);
        walk(db, &id, &["doing"], None);
    }
    let (id, _) = new_item(db, &project_id, "Checked", Priority::P2);
    walk(db, &id, &["doing", "review", "verify"], None);
    new_item(db, &project_id, "Fresh", Priority::P2);
    let mut owner = WsClient::connect(&pair.d).await;

    let o = overview(&mut owner).await;
    let r = row(&o, &project_id);
    assert_eq!(r["doing"].as_array().unwrap().len(), 3, "{r}");
    assert_eq!(r["doing_total"], 4, "{r}");
    let counts: Vec<(&str, u64)> = r["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["key"].as_str().unwrap(),
                c.get("count").map_or(0, |n| n.as_u64().unwrap()),
            )
        })
        .filter(|(_, n)| *n > 0)
        .collect();
    assert_eq!(counts, [("inbox", 1), ("doing", 4), ("verify", 1)], "{r}");
    assert_eq!(r["columns"][0]["name"], "Inbox", "{r}");
}

#[tokio::test]
async fn a_run_card_adds_one_owner_action_row() {
    let (pair, mut bots) = project_with_bots(&["Lead"]).await;
    let project_id = pair
        .d
        .app
        .db
        .get_bot(&pair.ids[0])
        .unwrap()
        .unwrap()
        .project_id;
    let dir = tempfile::tempdir().unwrap();
    let proposed = bots[0]
        .call(
            "propose_owner_action",
            json!({"content": "echo hi", "reason": "check the install",
                   "cwd": dir.path().display().to_string()}),
        )
        .await;
    assert!(proposed["owner_action"]["id"].is_string(), "{proposed}");
    let mut owner = WsClient::connect(&pair.d).await;
    let o = overview(&mut owner).await;
    let by_kind = &row(&o, &project_id)["attention"]["by_kind"];
    assert_eq!(by_kind["owner_action"], 1, "{o}");
    assert_eq!(row(&o, &project_id)["attention"]["score"], 3);
}

/// CE-014 S1: a prompt raised through the hook is one row whose target is
/// the request id the phone answers with, and it leaves once answered. Its
/// title carries the summary with token-like values masked (F1).
#[tokio::test]
async fn a_permission_prompt_via_the_hook_is_one_row_until_answered() {
    let (pair, _bots) = project_with_bots(&["Lead"]).await;
    let bot_id = pair.ids[0].clone();
    let project_id = pair.d.app.db.get_bot(&bot_id).unwrap().unwrap().project_id;
    let mut owner = WsClient::connect(&pair.d).await;
    let token = pair.d.app.secrets.bot_token(&bot_id).expect("token");
    let url = format!("http://{}/hook/permission", pair.d.addr);
    let hook = tokio::spawn(async move {
        reqwest::Client::new()
            .post(url)
            .bearer_auth(token)
            .json(&json!({
                "hook_event_name": "PermissionRequest",
                "session_id": "s", "transcript_path": "/t.jsonl", "cwd": "/w",
                "tool_name": "Bash",
                "tool_input": {"command": "curl -H 'Authorization: Bearer s3cr3t' https://x"},
            }))
            .send()
            .await
            .expect("hook post")
            .status()
            .as_u16()
    });
    let card = owner.wait_for(|v| v["type"] == "permission_request").await;
    let request_id = card["request"]["id"].as_str().expect("id").to_string();
    // The summary is masked; the card's full input is the owner's to judge.
    assert_eq!(
        card["request"]["summary"],
        "Bash: curl -H 'Authorization: Bearer ***' https://x"
    );

    let rows = owner
        .request(json!({"type": "attention_rows", "project_id": project_id}))
        .await;
    let prompts: Vec<&Value> = rows["attention_rows"]["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .filter(|r| r["kind"] == "PERMISSION_PROMPT")
        .collect();
    assert_eq!(prompts.len(), 1, "{rows}");
    assert_eq!(prompts[0]["request_id"], request_id.as_str());
    assert_eq!(prompts[0]["weight"], 3);
    assert_eq!(
        prompts[0]["title"],
        "Lead asks: Bash: curl -H 'Authorization: Bearer ***' https://x"
    );
    let o = overview(&mut owner).await;
    assert_eq!(
        row(&o, &project_id)["attention"]["by_kind"]["permission_prompt"],
        1
    );

    let answered = owner
        .request(json!({"type": "answer_permission", "request_id": request_id, "decision": "deny"}))
        .await;
    assert_eq!(answered["type"], "permission", "{answered}");
    assert_eq!(hook.await.expect("hook task"), 200);
    let rows = owner
        .request(json!({"type": "attention_rows", "project_id": project_id}))
        .await;
    assert!(
        !rows.to_string().contains("PERMISSION_PROMPT"),
        "the answered prompt is gone: {rows}"
    );
}

#[tokio::test]
async fn json_and_binary_answer_the_same_overview() {
    let (pair, _bots) = project_with_bots(&["Lead"]).await;
    let mut owner = WsClient::connect(&pair.d).await;
    let json_answer: ProjectsOverview =
        serde_json::from_value(overview(&mut owner).await).expect("proto3 JSON decodes");
    let req_id = 77;
    let request = Envelope {
        req_id,
        body: Some(Body::HomeRequest(HomeRequest {
            request: Some(Request::ProjectsOverview(ProjectsOverviewRequest::default())),
        })),
    };
    let started = Instant::now();
    send_raw(&mut owner, request.encode_to_vec()).await;
    let envelope = next_envelope(&mut owner, Duration::from_secs(5))
        .await
        .expect("an answer");
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(envelope.req_id, req_id);
    let Some(Body::HomeResponse(response)) = envelope.body else {
        panic!("expected a home response");
    };
    let Some(Response::ProjectsOverview(binary)) = response.response else {
        panic!("expected the overview");
    };
    // Only the time it was read differs.
    assert_eq!(binary.rows, json_answer.rows);
    assert_eq!(binary.sources, json_answer.sources);
    assert_eq!(binary.total, json_answer.total);
}

#[tokio::test]
async fn reading_the_overview_needs_the_read_grant() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let created = owner
        .request(json!({"type": "create_device", "name": "remote", "capabilities": ["control"]}))
        .await;
    let mut device = WsClient::connect_as(&d, token_str(&created)).await;
    for request in ["projects_overview", "attention_rows"] {
        let reply = device
            .request(json!({"type": request, "project_id": "p"}))
            .await;
        assert_eq!(reply["code"], "forbidden", "{request}: {reply}");
    }
    let reply = device
        .request(json!({"type": "attention_dismiss", "id": "decision:x:y"}))
        .await;
    assert_eq!(reply["code"], "forbidden", "dismissing is approve's");
    let hello = raw_hello(&d, token_str(&created)).await;
    let capabilities = hello["capabilities"].as_array().unwrap();
    assert!(
        capabilities.contains(&json!("projects_overview")),
        "{hello}"
    );
    assert_eq!(hello["contracts"]["home"], 1);
}

#[tokio::test]
async fn changes_are_pushed_debounced() {
    let (pair, _bots) = project_with_bots(&["Lead"]).await;
    let app = &pair.d.app;
    let project_id = app.db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    let mut owner = WsClient::connect(&pair.d).await;
    let is_change = |v: &Value| v["type"] == "projects_overview_changed";
    let bot = pair.ids[0].clone();
    wait_until("the bot is up", || {
        app.supervisor.state(&bot).0 != BotState::Stopped
    })
    .await;
    // A bot going working: one push within 3 s (D4 a).
    let started = Instant::now();
    app.supervisor.set_state(&bot, BotState::Working, "");
    let push = owner
        .wait_for_within(Duration::from_secs(3), is_change)
        .await;
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(
        push["project_ids"]
            .as_array()
            .unwrap()
            .contains(&json!(project_id)),
        "{push}"
    );
    // A burst of 20 events: at most 2 pushes (D4 b).
    for n in 0..20 {
        let state = if n % 2 == 0 {
            BotState::Ready
        } else {
            BotState::Working
        };
        app.supervisor.set_state(&bot, state, "");
    }
    let mut pushes = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(4500);
    while let Ok(Some(Ok(frame))) =
        tokio::time::timeout_at(deadline, futures::StreamExt::next(&mut owner.rx)).await
    {
        if let tokio_tungstenite::tungstenite::Message::Text(text) = frame {
            let v: Value = serde_json::from_str(&text).unwrap();
            pushes += usize::from(is_change(&v));
        }
    }
    assert!((1..=2).contains(&pushes), "{pushes} pushes");
}
