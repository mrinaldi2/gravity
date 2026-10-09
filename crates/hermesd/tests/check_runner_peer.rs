//! The check runner across linked computers (H-283, H-261 §1.6, §7): a
//! computer's probed tools reach the board's home over the peer link, and a
//! check pinned to `windows` is run by the Windows computer's daemon, in a
//! checkout at the exact sha cloned from its own URL for the repository and
//! removed once run; the result, from the exit status, reaches the home.
//! The hands-on run on win-pc is Tester Win's.

mod common;

use std::collections::BTreeMap;

use common::peer_board::board;
use common::peers::wait_until;
use common::repo::{git, remote};
use hermesd::board::release::machines;
use hermesd::machine_tools as tool_probe;
use hermesd::prs::check_jobs::{dispatch, SYSTEM_RUNNER};
use hermesd::prs::check_model::{CheckResult, NewCheck};

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
    let sha = git(&origin, &["rev-parse", "main"]).trim().to_string();
    let tree = git(&origin, &["rev-parse", "main^{tree}"])
        .trim()
        .to_string();
    let check = NewCheck {
        name: "windows".into(),
        run: "git rev-parse HEAD".into(),
        needs: vec!["cargo".into()],
        machine: Some("windows".into()),
        required: true,
        result: CheckResult::Queued,
        note: None,
    };
    db.board_tx(|t| t.queue_checks(&b.mac_app, &url, &sha, &tree, &[check]))
        .unwrap();

    let run = || {
        db.board_read(|t| t.check_run(&b.mac_app, &sha, "windows"))
            .unwrap()
            .unwrap()
    };
    // The PC's project has no such repository: the PC refuses, it waits.
    dispatch(&mac.app, &b.mac_app).await.unwrap();
    assert!(run().runner.is_none());
    assert!(run().note.unwrap().contains("repositories"), "{:?}", run());
    assert_eq!(db.board_read(|t| t.open_jobs_on("win")).unwrap(), 0);

    win.app
        .db
        .set_project_repo(
            &b.win_app,
            Some(&bus::ProjectRepo {
                url,
                branch: "main".into(),
            }),
        )
        .unwrap();
    dispatch(&mac.app, &b.mac_app).await.unwrap();
    assert_eq!(
        run().runner.as_deref(),
        Some(SYSTEM_RUNNER),
        "no bot runs it"
    );
    assert!(mac
        .app
        .db
        .project_workers(&b.mac_app, 10)
        .unwrap()
        .is_empty());
    // A clone and a first start of the runner: longer than wait_until's.
    for _ in 0..1800 {
        if run().result.is_final() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let done = run();
    assert_eq!(done.result, CheckResult::Pass, "{done:?}");
    assert_eq!(done.ran_on.as_deref(), Some("win"));
    let log = done.log_artifact.unwrap();
    let path = log.strip_prefix("win:").expect("the log stays on the PC");
    assert!(std::fs::read_to_string(path).unwrap().contains(&sha));
    let job = std::path::Path::new(path).parent().unwrap();
    assert!(!job.join("check").exists(), "the PC removes its checkout");
    assert_eq!(db.board_read(|t| t.open_jobs_on("win")).unwrap(), 0);
}
