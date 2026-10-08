//! The dashboard's Needs you across linked computers (H-112, H-178). It lists
//! what the projects home counts: this computer's rows, and each linked
//! computer's last good part, marked with where to act on it. With a
//! computer away its rows stay, and the dashboard says it may not be all.

mod common;

use std::time::Duration;

use common::peer_board::board;
use common::peers::{bot_named, wait_until};
use common::{McpClient, WsClient};
use serde_json::{json, Value};

/// The dashboard, read again until `ready` holds (a linked computer's part
/// arrives in the background), for at most five seconds.
async fn dashboard_when(
    client: &mut WsClient,
    project_id: &str,
    ready: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let reply = client
            .request(json!({"type": "dashboard_get", "project_id": project_id, "all_kinds": true}))
            .await;
        assert_eq!(reply["type"], "dashboard", "{reply}");
        let d = reply["dashboard"].clone();
        if ready(&d) || tokio::time::Instant::now() > deadline {
            return d;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `(title, where)` of each decision: this computer's own (`None`), or a
/// linked computer's, by its name.
fn decisions(d: &Value) -> Vec<(String, Option<String>)> {
    let rows = d["needs_you"].as_array().cloned().unwrap_or_default();
    rows.iter()
        .filter_map(|row| match row["kind"].as_str() {
            Some("decision") => Some((row["title"].as_str()?.to_string(), None)),
            Some("elsewhere") if row["row"]["kind"] == "decision" => Some((
                row["row"]["title"].as_str()?.to_string(),
                row["elsewhere"].as_str().map(str::to_string),
            )),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn needs_you_off_home_shows_the_homes_rows_and_keeps_them_while_it_is_away() {
    let mut b = board().await;
    let lead = bot_named(&b.p.mac, &b.mac_app, "lead").expect("lead");
    let token = b.p.mac.app.secrets.bot_token(&lead.id).expect("token");
    let mut mac_lead = McpClient::new(&b.p.mac, &token);
    mac_lead
        .call(
            "raise_decision",
            json!({"title": "Freeze on Friday?", "body": "Yes or no."}),
        )
        .await;
    b.tester
        .call(
            "raise_decision",
            json!({"title": "Which PC test plan?", "body": "Pick one."}),
        )
        .await;

    let win_app = b.win_app.clone();
    let has_home_row = |d: &Value| decisions(d).iter().any(|(t, _)| t == "Freeze on Friday?");
    let d = dashboard_when(&mut b.p.win_client, &win_app, has_home_row).await;
    let rows = decisions(&d);
    let home = d["home"].as_str().expect("home").to_string();
    assert!(
        rows.contains(&("Freeze on Friday?".into(), Some(home.clone()))),
        "{rows:?}"
    );
    assert!(
        rows.contains(&("Which PC test plan?".into(), None)),
        "{rows:?}"
    );
    assert_eq!(d["needs_you_note"], Value::Null);
    assert_eq!(d["needs_you_count"], 2, "{d}");

    // The Mac goes away: its last good rows stay, and the note says so.
    let mac_peer_id = b.p.mac_peer_id.clone();
    b.p.mac.app.db.revoke_peer(&mac_peer_id).unwrap();
    b.p.mac.app.peers.disconnect(&mac_peer_id);
    let (win, win_peer_id) = (&b.p.win, b.p.win_peer_id.clone());
    wait_until("the PC loses the Mac", || {
        !win.app.peers.is_online(&win_peer_id)
    })
    .await;
    let noted = |d: &Value| d["needs_you_note"].is_string();
    let d = dashboard_when(&mut b.p.win_client, &win_app, noted).await;
    assert_eq!(decisions(&d).len(), 2, "{d}");
    assert!(
        d["needs_you_note"].as_str().is_some_and(|n| n
            .ends_with(" right now, so this may not be everything that needs you.")
            && n.starts_with("Can't reach ")),
        "{d}"
    );
}

/// ARCH-R65: a permission prompt on another linked computer showed on the
/// projects home's card but not on the dashboard, which listed only this
/// computer and the board's home. Both now read one source.
#[tokio::test]
async fn a_prompt_on_a_linked_computer_shows_on_the_home_and_the_dashboard() {
    let mut b = board().await;
    // The PC's tester stops on a permission prompt; the PC's app has it.
    let token = b.p.win.app.secrets.bot_token(&b.tester_id).expect("token");
    let url = format!("http://{}/hook/permission", b.p.win.addr);
    let hook = tokio::spawn(async move {
        reqwest::Client::new()
            .post(url)
            .bearer_auth(token)
            .json(&json!({
                "hook_event_name": "PermissionRequest",
                "session_id": "s", "transcript_path": "/t.jsonl", "cwd": "/w",
                "tool_name": "Bash", "tool_input": {"command": "cargo test"},
            }))
            .send()
            .await
    });
    let card =
        b.p.win_client
            .wait_for(|v| v["type"] == "permission_request")
            .await;
    let request_id = card["request"]["id"].as_str().expect("id").to_string();

    // The Mac, the board's home: the prompt is on its home card…
    let mac_app = b.mac_app.clone();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let card = loop {
        let o =
            b.p.mac_client
                .request(json!({"type": "projects_overview"}))
                .await;
        let row = o["overview"]["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["project_id"] == mac_app.as_str())
            .cloned();
        if let Some(row) = row.filter(|r| r["attention"]["by_kind"]["permission_prompt"] == 1) {
            break row;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never on the home: {o}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    // …and on its dashboard, marked with the computer to answer it on.
    let is_prompt =
        |row: &Value| row["kind"] == "elsewhere" && row["row"]["kind"] == "permission_prompt";
    let d = dashboard_when(&mut b.p.mac_client, &mac_app, |d| {
        d["needs_you"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(is_prompt))
    })
    .await;
    let rows = d["needs_you"].as_array().expect("needs_you");
    let prompt = rows
        .iter()
        .find(|r| is_prompt(r))
        .unwrap_or_else(|| panic!("{d}"));
    assert_eq!(prompt["row"]["request_id"], request_id.as_str(), "{prompt}");
    assert!(prompt["elsewhere"].is_string(), "{prompt}");
    assert_eq!(
        d["needs_you_count"], card["attention"]["count"],
        "the header and the home card agree"
    );

    // Answered on the PC, it leaves the dashboard too.
    let answered = b
        .p
        .win_client
        .request(json!({"type": "answer_permission", "request_id": request_id, "decision": "deny"}))
        .await;
    assert_eq!(answered["type"], "permission", "{answered}");
    let _ = hook.await;
    let gone = |d: &Value| {
        !d["needs_you"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(is_prompt))
    };
    let d = dashboard_when(&mut b.p.mac_client, &mac_app, gone).await;
    assert!(gone(&d), "{d}");
}

/// ARCH-R70 M1: a linked computer on 0.17.2 or earlier still asks the home
/// `dashboard_needs_you`; the home answers it for one release (H-185 removes
/// it), with its rows in the asking computer's ids.
#[tokio::test]
async fn a_0_17_2_peer_still_gets_the_homes_rows() {
    let b = board().await;
    let lead = bot_named(&b.p.mac, &b.mac_app, "lead").expect("lead");
    let token = b.p.mac.app.secrets.bot_token(&lead.id).expect("token");
    let mut mac_lead = McpClient::new(&b.p.mac, &token);
    mac_lead
        .call(
            "raise_decision",
            json!({"title": "Freeze on Friday?", "body": "Yes or no."}),
        )
        .await;

    // The PC asks as 0.17.2 did: its own project id.
    let frame = json!({"type": "dashboard_needs_you", "project_id": b.win_app, "all_kinds": false});
    let answer =
        b.p.win
            .app
            .peers
            .request(&b.p.win_peer_id, frame)
            .await
            .expect("still served");
    let rows = answer["rows"].as_array().expect("rows");
    assert!(
        rows.iter()
            .any(|r| r["kind"] == "decision" && r["title"] == "Freeze on Friday?"),
        "{answer}"
    );
    assert!(answer["count"].as_u64().is_some_and(|n| n >= 1), "{answer}");
    assert!(answer["wip_overrides"].is_array(), "{answer}");
}
