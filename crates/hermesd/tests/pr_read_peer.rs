//! PRs from a linked computer (H-273; H-261 §12.9, B9): the board and its
//! PRs are on the Mac. The PC's tester opens a PR through `board_call`; the
//! PC's app reads it through its own daemon in its own project, and gets the
//! Mac's pushes there. The owner's re-run is forwarded to the Mac (H-285).

mod common;

use bus::contract::pr::{self as p, pr_push::Push, pr_response::Response};
use common::peer_board::board;
use common::peers::wait_until;
use common::pr_wire::{self as w, call, error, expect_push, response};
use common::prs::{clone, commit};
use common::repo::{git, remote};
use common::WsClient;
use hermesd::actor::Actor;
use hermesd::db::MoveTo;
use serde_json::json;

#[tokio::test]
async fn a_linked_computer_reads_prs_and_gets_their_pushes() {
    let mut b = board().await;
    let dir = tempfile::tempdir().unwrap();
    let origin = remote(dir.path());
    let tree = dir.path().join("tree");
    clone(&origin, &tree, "H-1-peer");
    let head = commit(&tree, "peer.txt", "one\ntwo\n");
    git(&tree, &["push", "-q", "origin", "H-1-peer"]);
    let mac = &b.p.mac.app.db;
    mac.set_project_repo(
        &b.mac_app,
        Some(&bus::ProjectRepo {
            url: origin.display().to_string(),
            branch: "main".into(),
        }),
    )
    .unwrap();
    let item = mac.get_item(&b.item).unwrap().unwrap();
    let to = MoveTo {
        column: "doing",
        ..MoveTo::default()
    };
    mac.move_item(&item.id, item.version, &to, &Actor::User)
        .unwrap();

    // The PC's tester opens it on the Mac, through `board_call`.
    let opened = b
        .tester
        .call("pr_open", json!({"item": b.item, "branch": "H-1-peer"}))
        .await;
    assert_eq!(opened["pr"]["number"], 1, "{opened}");

    let mut pc = WsClient::connect(&b.p.win).await;
    let prs = w::list(&mut pc, &b.win_app, &[]).await;
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].project_id, b.win_app, "named in the PC's project");
    assert_eq!(prs[0].head_sha, head);
    let pr = w::pr(&mut pc, &b.win_app, 1).await;
    assert_eq!(
        (pr.project_id.as_str(), pr.item_id.as_str()),
        (b.win_app.as_str(), b.item.as_str())
    );
    let Response::PrDiff(diff) =
        response(call(&mut pc, w::diff(&b.win_app, None, None, None)).await)
    else {
        panic!("a diff");
    };
    assert_eq!(diff.files[0].path, "peer.txt");
    assert_eq!(diff.files[0].additions, 2);

    // A change on the Mac reaches the PC's app, in the PC's project.
    b.tester
        .call(
            "pr_comment",
            json!({"number": 1, "sha": head, "path": "peer.txt", "line": 1, "body": "Hi."}),
        )
        .await;
    expect_push(
        &mut pc,
        Push::PrUpdated(p::PrUpdated {
            project_id: b.win_app.clone(),
            number: 1,
        }),
    )
    .await;

    // The owner's re-run goes to the board's home (H-285): its answer.
    let (code, message) = error(call(&mut pc, w::rerun(&b.win_app, &head, "rust")).await);
    assert_eq!(code, "not_found", "{message}");

    // While the Mac is away, the PC says so.
    let mac_peer_id = b.p.mac_peer_id.clone();
    b.p.mac.app.db.revoke_peer(&mac_peer_id).unwrap();
    b.p.mac.app.peers.disconnect(&mac_peer_id);
    let (win, win_peer_id) = (&b.p.win, b.p.win_peer_id.clone());
    wait_until("the PC loses the Mac", || {
        !win.app.peers.is_online(&win_peer_id)
    })
    .await;
    let (code, _) = error(call(&mut pc, w::get(&b.win_app, 1)).await);
    assert_eq!(code, "unavailable");
}
