//! A bot stopped on a permission prompt only its terminal shows is on the
//! owner's Needs you (H-172), counted in its project's overview, until the
//! prompt is answered.

mod common;

use bus::BotState;
use common::peers::project;
use common::*;
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

/// A bot stopped on a permission prompt in its own terminal, with no app
/// to show a card, is a Needs-you row (H-172): "<bot> needs approval" with
/// the masked command, weighed as an ordinary decision, until a tool runs.
#[tokio::test]
async fn a_bot_waiting_on_its_terminal_prompt_is_a_row_until_approved() {
    let d = spawn_daemon().await;
    // An owner app that shows no cards: the terminal asks.
    let token = match d.app.secrets.client_token() {
        "" => d.app.owner.mint(),
        token => token.to_string(),
    };
    let mut owner = WsClient::connect_with(&d, &token, &[]).await;
    let project_id = project(&mut owner, "p").await;
    let bot = create_bot(&mut owner, &project_id, "Lead").await;
    let bot_id = bot["id"].as_str().expect("id").to_string();
    owner
        .wait_for(|v| v["type"] == "bot_state" && v["state"] == "ready")
        .await;
    let bot_token = d.app.secrets.bot_token(&bot_id).expect("token");
    let http = reqwest::Client::new();
    let post = |path: &str, body: Value| {
        http.post(format!("http://{}{path}", d.addr))
            .bearer_auth(bot_token.clone())
            .json(&body)
            .send()
    };
    let needs_you = |rows: &Value| -> Vec<Value> {
        // An empty list is left out of the answer (proto3 JSON).
        rows["attention_rows"]["rows"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|r| r["kind"] == "BOT_WAITING")
            .cloned()
            .collect()
    };

    let asked = post(
        "/hook/permission",
        json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "s", "transcript_path": "/t.jsonl", "cwd": "/w",
            "tool_name": "Bash",
            "tool_input": {"command": "curl -H 'Authorization: Bearer s3cr3t' https://x"},
        }),
    )
    .await
    .expect("hook post");
    assert_eq!(asked.status().as_u16(), 200);
    assert_eq!(asked.text().await.expect("body"), "", "the terminal asks");
    post(
        "/hook",
        json!({"event": "Notification", "message": "Claude needs your permission to use Bash"}),
    )
    .await
    .expect("hook post");
    assert_eq!(
        d.app.supervisor.state(&bot_id).0,
        BotState::WaitingForApproval
    );

    let rows = owner
        .request(json!({"type": "attention_rows", "project_id": project_id}))
        .await;
    let waiting = needs_you(&rows);
    assert_eq!(waiting.len(), 1, "{rows}");
    assert_eq!(
        waiting[0]["title"],
        "Lead needs approval: Bash: curl -H 'Authorization: Bearer ***' https://x"
    );
    assert!(!rows.to_string().contains("s3cr3t"), "{rows}");
    assert_eq!(waiting[0]["weight"], 1, "ranked as an ordinary decision");
    assert_eq!(waiting[0]["bot"]["bot_id"], bot_id.as_str());
    let o = overview(&mut owner).await;
    let attention = &row(&o, &project_id)["attention"];
    assert_eq!(attention["by_kind"]["bot_waiting"], 1, "{o}");
    assert_eq!(attention["count"], 1, "{o}");
    assert_eq!(o["total"]["count"], 1, "{o}");

    // Approving in the terminal runs the tool: the row is gone.
    post("/hook", json!({"event": "PostToolUse"}))
        .await
        .expect("hook post");
    let rows = owner
        .request(json!({"type": "attention_rows", "project_id": project_id}))
        .await;
    assert!(needs_you(&rows).is_empty(), "{rows}");
    let o = overview(&mut owner).await;
    assert!(
        row(&o, &project_id)["attention"].get("count").is_none(),
        "{o}"
    );

    // A notification alone names no command.
    post(
        "/hook",
        json!({"event": "Notification", "message": "Claude needs your permission to use Bash"}),
    )
    .await
    .expect("hook post");
    let rows = owner
        .request(json!({"type": "attention_rows", "project_id": project_id}))
        .await;
    assert_eq!(
        needs_you(&rows)[0]["title"],
        "Lead needs approval",
        "{rows}"
    );
}
