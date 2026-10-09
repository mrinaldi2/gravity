//! The check runner across linked computers (H-283, H-261 §1.6, §7): a
//! computer's probed tools reach the board's home over the peer link, and a
//! check pinned to `windows` gets a worker on the Windows computer, whose
//! daemon makes the checkout at the exact sha and removes it once the
//! report has reached the home. The hands-on run on win-pc is Tester Win's.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use common::peer_board::board;
use common::peers::wait_until;
use common::repo::{git, remote};
use common::McpClient;
use hermesd::board::model::{ProjectRole, Role};
use hermesd::board::release::machines;
use hermesd::machine_tools as tool_probe;
use hermesd::prs::check_jobs::dispatch;
use hermesd::prs::check_model::{CheckResult, NewCheck};
use serde_json::json;

#[tokio::test]
async fn a_windows_check_runs_on_the_linked_windows_computer() {
    let b = board().await;
    let (mac, win) = (&b.p.mac, &b.p.win);

    // The PC's probe reaches the Mac under the name it knows it by.
    tool_probe::probe_and_share(&win.app).await.unwrap();
    let probed = || mac.app.db.board_read(|t| t.machine_tools("win")).unwrap();
    wait_until("the PC's tools reach the Mac", || {
        probed().contains_key("os")
    })
    .await;
    // As a Windows computer with cargo, sent again as a new link would.
    let tools: BTreeMap<String, String> = [
        ("cargo", "1.90.0"),
        ("os", "windows"),
        ("disk_free_gb", "200"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let pc = win.app.db.board_read(machines::this_computer).unwrap();
    win.app
        .db
        .board_tx(|t| t.set_machine_tools(&pc, &tools))
        .unwrap();
    tool_probe::link_up(&win.app, &b.p.win_peer_id);
    wait_until("the Mac sees a Windows computer", || probed() == tools).await;

    // The Mac's project has a repository, a lead, and a check on a commit.
    let dir = tempfile::tempdir().unwrap();
    let origin = remote(dir.path());
    let db = &mac.app.db;
    let url = origin.display().to_string();
    db.set_project_repo(
        &b.mac_app,
        Some(&bus::ProjectRepo {
            url: url.clone(),
            branch: "main".into(),
        }),
    )
    .unwrap();
    let lead = common::peers::bot_named(mac, &b.mac_app, "lead").unwrap();
    db.set_project_role(&ProjectRole {
        project_id: b.mac_app.clone(),
        role: Role::Lead,
        bot_id: lead.id.clone(),
        machine: None,
    })
    .unwrap();
    let sha = git(&origin, &["rev-parse", "main"]).trim().to_string();
    let tree = git(&origin, &["rev-parse", "main^{tree}"])
        .trim()
        .to_string();
    let check = NewCheck {
        name: "windows".into(),
        run: "cargo test --workspace -j 2".into(),
        needs: vec!["cargo".into()],
        machine: Some("windows".into()),
        required: true,
        result: CheckResult::Queued,
        note: None,
    };
    db.board_tx(|t| t.queue_checks(&b.mac_app, &url, &sha, &tree, &[check]))
        .unwrap();

    dispatch(&mac.app, &b.mac_app).await.unwrap();
    let run = db
        .board_read(|t| t.check_run(&b.mac_app, &sha, "windows"))
        .unwrap()
        .unwrap();
    let stand_in = db
        .get_bot(run.runner.as_deref().expect("dispatched"))
        .unwrap()
        .unwrap();
    assert!(stand_in.is_linked(), "the runner is a worker on the PC");
    assert_eq!(db.board_read(|t| t.open_jobs_on("win")).unwrap(), 1);

    let bot = win
        .app
        .db
        .get_bot(stand_in.remote_bot_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    let workspace = Path::new(&bot.workspace_path).to_path_buf();
    let checkout = workspace.join("check");
    for _ in 0..300 {
        if checkout.join(".git").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]).trim(), sha);

    let token = win.app.secrets.bot_token(&bot.id).expect("token");
    let mut worker = McpClient::new(win, &token);
    std::fs::write(workspace.join("check.log"), "ok\n").unwrap();
    let done = worker
        .call(
            "check_report",
            json!({"sha": sha, "name": "windows", "result": "pass", "log": "check.log"}),
        )
        .await;
    assert_eq!(done["check"]["result"], "pass", "{done}");
    assert_eq!(done["check"]["ran_on"], "win");
    assert!(
        !checkout.exists(),
        "the PC removes its checkout once reported"
    );
    assert_eq!(db.board_read(|t| t.open_jobs_on("win")).unwrap(), 0);
}
