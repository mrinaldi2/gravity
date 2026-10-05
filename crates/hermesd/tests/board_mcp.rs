//! The board tools end to end over MCP (B5): role-filtered lists, schemas
//! from the contract with short enum names, guarded moves and refusals, and
//! the bus tools' `item` links with task history.

mod common;

use common::board::version;
use common::tasks::{error_text, project_with_bots};
use common::McpClient;
use serde_json::{json, Value};

fn tool_names(list: &Value) -> Vec<String> {
    list["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_string())
        .collect()
}

async fn get(bot: &mut McpClient, id: &str) -> Value {
    bot.call("item_get", json!({ "id": id })).await
}

#[tokio::test]
async fn bots_work_an_item_through_the_board_over_mcp() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "Desktop Dev", "Tester"]).await;
    let [lead, dev, tester] = &mut bots[..] else {
        unreachable!()
    };
    let project = pair
        .d
        .app
        .db
        .get_bot(&pair.ids[0])
        .unwrap()
        .unwrap()
        .project_id;

    // No board yet: the tools its seeded role will have are listed already
    // (a session reads the list once), and calling one says why.
    assert!(tool_names(&dev.tools().await).contains(&"item_move".to_string()));
    let raw = dev.call_raw("board_get", json!({})).await;
    assert!(error_text(&raw).contains("no board"));

    pair.d
        .app
        .db
        .ensure_board(&project, &pair.d.app.db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    let dev_tools = tool_names(&dev.tools().await);
    assert!(dev_tools.contains(&"item_move".to_string()));
    assert!(!dev_tools.contains(&"item_assign".to_string()));
    assert!(tool_names(&lead.tools().await).contains(&"item_assign".to_string()));
    assert!(tool_names(&tester.tools().await).contains(&"item_check_ac".to_string()));
    let list = dev.tools().await;
    let create = list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "item_create")
        .unwrap();
    assert_eq!(
        create["inputSchema"]["properties"]["type"]["enum"],
        json!(["epic", "feature", "bug", "spike", "chore"])
    );

    let created = dev
        .call(
            "item_create",
            json!({"type": "feature", "title": "Board tools", "platforms": ["daemon"],
                   "size": "M", "acceptance_criteria": ["bots can move cards"]}),
        )
        .await;
    let item = &created["item"];
    let id = item["id"].as_str().unwrap().to_string();
    assert_eq!(
        (item["type"].as_str(), item["category"].as_str()),
        (Some("feature"), Some("inbox"))
    );

    // Only the lead assigns.
    let raw = dev
        .call_raw(
            "item_assign",
            json!({"id": id, "expected_version": version(item), "bot": "Desktop Dev"}),
        )
        .await;
    assert!(error_text(&raw).contains("board role"));

    // The lead refines: the DoR wants a spec link first, and says so.
    let raw = lead
        .call_raw(
            "item_move",
            json!({"id": id, "to": "ready", "expected_version": version(item)}),
        )
        .await;
    assert!(
        error_text(&raw).contains("dor.spec_link_for_ui_or_daemon"),
        "{raw}"
    );
    lead.call(
        "item_link",
        json!({"id": id, "kind": "artifact", "ref": "artifacts/spec.md"}),
    )
    .await;
    let item = get(lead, &id).await["item"].clone();
    let ready = lead
        .call(
            "item_move",
            json!({"id": id, "to": "ready", "expected_version": version(&item)}),
        )
        .await;
    assert_eq!(ready["item"]["column_key"], "ready");

    // A stale version is a conflict that names the current one.
    let raw = lead
        .call_raw(
            "item_assign",
            json!({"id": id, "expected_version": version(&item), "bot": "Desktop Dev"}),
        )
        .await;
    assert!(error_text(&raw).contains("conflict"));
    let assigned = lead
        .call(
            "item_assign",
            json!({"id": id, "expected_version": version(&ready["item"]), "bot": "Desktop Dev"}),
        )
        .await;

    // The lead delegates with `item`: the task is linked, and its completion
    // lands in the item's history with the artifacts.
    let sent = lead
        .call(
            "send_message",
            json!({"to": "Desktop Dev", "kind": "task", "body": "build it", "item": id}),
        )
        .await;
    let task_id = sent["task_id"].as_str().unwrap().to_string();
    let moved = dev
        .call(
            "item_move",
            json!({"id": id, "to": "doing", "expected_version": version(&assigned["item"])}),
        )
        .await;
    assert_eq!(moved["item"]["category"], "doing");
    let raw = dev
        .call_raw(
            "item_move",
            json!({"id": id, "to": "review", "expected_version": version(&moved["item"])}),
        )
        .await;
    let text = error_text(&raw);
    assert!(
        text.contains("No branch is linked") && text.contains("Fix: Link the branch."),
        "{text}"
    );

    dev.call(
        "complete_task",
        json!({"task_id": task_id, "result": "done", "artifacts": ["artifacts/B5-change.md"]}),
    )
    .await;
    let full = get(dev, &id).await;
    let links: Vec<(&str, &str)> = full["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["kind"].as_str().unwrap(), l["ref"].as_str().unwrap()))
        .collect();
    assert!(links.contains(&("task", task_id.as_str())), "{links:?}");
    assert!(
        links.contains(&("artifact", "artifacts/B5-change.md")),
        "{links:?}"
    );
    let kinds: Vec<&str> = full["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"task_done"), "{kinds:?}");

    // A decision raised for the item is linked there.
    let raised = dev
        .call(
            "raise_decision",
            json!({"title": "Scope?", "body": "Which tools first?", "item": id}),
        )
        .await;
    let decision = raised["decision"]["id"].as_str().unwrap();
    let full = get(dev, &id).await;
    assert!(full["links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["ref"] == decision));

    // `item` must be an item of this project, and only on a task.
    let raw = dev
        .call_raw(
            "send_message",
            json!({"to": "Team Lead", "kind": "note", "body": "fyi", "item": id}),
        )
        .await;
    assert!(error_text(&raw).contains("send kind 'task'"));
    let raw = dev.call_raw("item_get", json!({"id": "X-404"})).await;
    assert!(error_text(&raw).contains("no item X-404"));

    // The check lists every other column for the asker, with short names.
    let check = dev.call("item_move_check", json!({"id": id})).await;
    let review = check["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["column"] == "review")
        .unwrap();
    assert_eq!(review["allowed"], false);
    let board = tester.call("board_get", json!({})).await;
    assert!(board["cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == id.as_str()));
}

#[tokio::test]
async fn bot_writes_over_mcp_reach_watching_boards() {
    use bus::contract::board::{self as c, board_request::Request};
    use common::board::{call, next_push};
    use common::WsClient;

    let (pair, mut bots) = project_with_bots(&["Team Lead", "Desktop Dev"]).await;
    let [lead, dev] = &mut bots[..] else {
        unreachable!()
    };
    let project = pair
        .d
        .app
        .db
        .get_bot(&pair.ids[0])
        .unwrap()
        .unwrap()
        .project_id;
    pair.d
        .app
        .db
        .ensure_board(&project, &pair.d.app.db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    let mut watcher = WsClient::connect(&pair.d).await;
    call(
        &mut watcher,
        Request::BoardWatch(c::BoardWatch {
            project_id: project.clone(),
        }),
    )
    .await;

    let created = dev
        .call(
            "item_create",
            json!({"type": "chore", "title": "Pushed", "platforms": ["infra"]}),
        )
        .await;
    let id = created["item"]["id"].as_str().unwrap().to_string();
    let event = next_push(&mut watcher)
        .await
        .expect("a push for the create");
    assert_eq!(
        (event.seq, event.kind),
        (1, c::BoardEventKind::ItemUpserted as i32)
    );
    assert_eq!(event.item_id, id);
    assert_eq!(event.card.expect("card").title, "Pushed");

    // A refusal writes nothing and pushes nothing; the next change is seq 2.
    let raw = dev
        .call_raw(
            "item_move",
            json!({"id": id, "to": "ready", "expected_version": version(&created["item"])}),
        )
        .await;
    error_text(&raw);
    let cancelled = lead
        .call(
            "item_move",
            json!({"id": id, "to": "cancelled", "reason": "not needed",
                   "expected_version": version(&created["item"])}),
        )
        .await;
    let event = next_push(&mut watcher).await.expect("a push for the move");
    assert_eq!(
        (event.seq, event.kind),
        (2, c::BoardEventKind::ItemMoved as i32)
    );
    assert_eq!(event.from_column.as_deref(), Some("inbox"));
    assert_eq!(
        event.card.expect("card").version,
        version(&cancelled["item"])
    );
}
