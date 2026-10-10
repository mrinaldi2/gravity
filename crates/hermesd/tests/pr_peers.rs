//! PRs from a linked computer, PR-9 (H-285; H-261 §1, §4.3, §12.9): the
//! board and its PRs are on the Mac. A bot on the PC opens, pushes, reviews
//! and comments through `board_call`, each record naming that bot's
//! stand-in, with its worktree checked on the PC and recorded as the PC's.
//! The owner's review, resolve and re-run from the PC's app reach the Mac
//! with how the owner proved it there; the PC's owner token and a bot
//! relaying for the owner are refused. (A check run on the PC, AC3, is
//! H-283's `check_runner_peer`.)

mod common;

use common::peer_board::{board, Board};
use common::peers::{bot_named, wait_until};
use common::pr_wire::{self as w, call, error};
use common::prs::{clone, commit};
use common::repo::{git, remote};
use common::{create_bot, McpClient, WsClient};
use hermesd::actor::Actor;
use hermesd::board::model::{ProjectRole, Role};
use hermesd::db::MoveTo;
use serde_json::json;

/// The Mac's project on a fresh repository, H-1 in Doing, and the PC's
/// tester's worktree on branch H-1-peer with one commit pushed: the tree and
/// its head.
fn ready(b: &Board, dir: &std::path::Path) -> (std::path::PathBuf, String) {
    let origin = remote(dir);
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
    // In the PC tester's own workspace: a folder that bot may work in.
    let ws =
        b.p.win
            .app
            .db
            .get_bot(&b.tester_id)
            .unwrap()
            .unwrap()
            .workspace_path;
    let tree = std::path::Path::new(&ws).join("H-1-peer");
    std::fs::create_dir_all(&ws).unwrap();
    clone(&origin, &tree, "H-1-peer");
    let head = commit(&tree, "peer.txt", "one\n");
    git(&tree, &["push", "-q", "origin", "H-1-peer"]);
    (tree, head)
}

fn on_mac(b: &Board, number: u32) -> hermesd::prs::model::Pr {
    let db = &b.p.mac.app.db;
    db.board_read(|t| t.pr(&b.mac_app, number))
        .unwrap()
        .unwrap()
}

/// A second bot on the PC, linked to the Mac's project, holding a board
/// role there: the bot and its stand-in.
async fn pc_bot(b: &mut Board, name: &str, role: Role) -> (McpClient, String) {
    let bot = create_bot(&mut b.p.win_client, &b.win_app, name).await;
    let id = bot["id"].as_str().unwrap().to_string();
    let (mac, app) = (&b.p.mac, b.mac_app.clone());
    wait_until("the bot stands in on the Mac", || {
        bot_named(mac, &app, name).is_some()
    })
    .await;
    let stand_in = bot_named(mac, &app, name).unwrap().id;
    mac.app
        .db
        .set_project_role(&ProjectRole {
            project_id: b.mac_app.clone(),
            role,
            bot_id: stand_in.clone(),
            machine: None,
        })
        .unwrap();
    let token = b.p.win.app.secrets.bot_token(&id).expect("token");
    (McpClient::new(&b.p.win, &token), stand_in)
}

/// AC1: open, push, review and comment from the PC, each naming that bot.
#[tokio::test]
async fn a_linked_bot_works_on_its_pr_from_its_own_computer() {
    let mut b = board().await;
    let dir = tempfile::tempdir().unwrap();
    let (tree, head) = ready(&b, dir.path());
    let path = tree.display().to_string();

    let raw = b
        .tester
        .call_raw(
            "pr_open",
            json!({"item": b.item, "branch": "H-1-peer", "worktree": "/nowhere/H-1-peer"}),
        )
        .await;
    assert!(
        raw["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("doesn't exist on"),
        "checked on the PC: {raw}"
    );
    let opened = b
        .tester
        .call(
            "pr_open",
            json!({"item": b.item, "branch": "H-1-peer", "worktree": path}),
        )
        .await;
    assert_eq!(opened["pr"]["number"], 1, "{opened}");
    let pr = on_mac(&b, 1);
    assert_eq!(pr.author, b.stand_in, "the PR is the tester's");
    let mac = &b.p.mac.app.db;
    let trees = mac.board_read(|t| t.pr_worktrees(&pr.id)).unwrap();
    let pc = mac.get_peer(&b.p.mac_peer_id).unwrap().unwrap().name;
    assert_eq!(trees.len(), 1, "{trees:?}");
    assert_eq!(
        (trees[0].machine.as_str(), trees[0].bot_id.as_str()),
        (pc.as_str(), b.stand_in.as_str()),
        "recorded as the PC's, for the tester"
    );
    assert!(trees[0].path.ends_with("H-1-peer"));

    let second = commit(&tree, "peer.txt", "one\ntwo\n");
    git(&tree, &["push", "-q", "origin", "H-1-peer"]);
    b.tester
        .call(
            "pr_push",
            json!({"number": 1, "sha": second, "worktree": path}),
        )
        .await;
    let pr = on_mac(&b, 1);
    assert_eq!(pr.head_sha, second);
    let pushes = mac.board_read(|t| t.pr_pushes(&pr.id)).unwrap();
    assert_eq!(
        pushes.last().unwrap().pushed_by.as_deref(),
        Some(b.stand_in.as_str())
    );
    assert_ne!(head, second);

    b.tester
        .call(
            "pr_comment",
            json!({"number": 1, "sha": second, "path": "peer.txt", "line": 2, "body": "Hm."}),
        )
        .await;
    let comments = mac.board_read(|t| t.comments(&pr.id)).unwrap();
    assert_eq!(
        comments[0].author, b.stand_in,
        "the comment is the tester's"
    );

    let (mut arch, arch_stand_in) = pc_bot(&mut b, "arch", Role::ReviewerArch).await;
    arch.call(
        "pr_review",
        json!({"number": 1, "sha": second, "role": "architect", "verdict": "approved",
               "summary": "Fine."}),
    )
    .await;
    let mac = &b.p.mac.app.db;
    let reviews = mac.board_read(|t| t.reviews(&pr.id)).unwrap();
    assert_eq!(
        reviews[0].reviewer, arch_stand_in,
        "the review is the architect's"
    );
}

/// AC2: the owner's acts from the PC's app reach the Mac with how the owner
/// proved it there; the PC's owner token and a bot relaying are refused.
#[tokio::test]
async fn the_owners_acts_from_a_linked_computer_carry_its_proof() {
    let mut b = board().await;
    let dir = tempfile::tempdir().unwrap();
    let (_, head) = ready(&b, dir.path());
    b.tester
        .call("pr_open", json!({"item": b.item, "branch": "H-1-peer"}))
        .await;
    let comment = b
        .tester
        .call(
            "pr_comment",
            json!({"number": 1, "sha": head, "path": "peer.txt", "line": 1, "body": "Hi.",
                   "severity": "nit"}),
        )
        .await;
    let comment_id = comment["comment"]["id"].as_str().unwrap().to_string();

    let review = json!({"type": "pr_review_submit", "project_id": b.win_app, "number": 1,
                        "sha": head, "verdict": "approved"});
    let mut token = WsClient::connect_owner_token(&b.p.win).await;
    assert_eq!(
        token.request(review.clone()).await["type"],
        "error",
        "the owner token"
    );
    let raw = b
        .tester
        .call_raw(
            "pr_review",
            json!({"number": 1, "sha": head, "role": "owner", "verdict": "approved"}),
        )
        .await;
    assert_eq!(
        raw["isError"], true,
        "a bot can't review as the owner: {raw}"
    );

    let mut app = WsClient::connect(&b.p.win).await;
    let out = app.request(review).await;
    assert_eq!(out["type"], "pr", "{out}");
    let pr = on_mac(&b, 1);
    let reviews = b.p.mac.app.db.board_read(|t| t.reviews(&pr.id)).unwrap();
    let owner = reviews
        .iter()
        .find(|r| r.role == "owner")
        .expect("recorded");
    let proof = format!("ticket@peer:{}", b.p.mac_peer_id);
    assert_eq!(
        owner.provenance.as_deref(),
        Some(proof.as_str()),
        "{owner:?}"
    );

    let resolved = app
        .request(
            json!({"type": "pr_comment_resolve", "project_id": b.win_app,
                        "number": 1, "comment_id": comment_id}),
        )
        .await;
    assert_eq!(resolved["comment"]["resolved"], true, "{resolved}");

    // The Mac takes the owner's act only with the PC's word on its proof: a
    // forwarded request without it is refused there too.
    let bare = json!({"type": "pr_owner", "project_id": b.win_app,
                      "kind": "pr_review_submit",
                      "request": {"number": 1, "sha": head, "verdict": "changes_requested",
                                  "summary": "No.", "findings": [{"severity": "must",
                                  "text": "No."}]}});
    let refused = b.p.win.app.peers.request(&b.p.win_peer_id, bare).await;
    let why = format!("{refused:?}");
    assert!(
        why.contains("only the owner's app or a paired device"),
        "{why}"
    );
    let reviews = b.p.mac.app.db.board_read(|t| t.reviews(&pr.id)).unwrap();
    assert_eq!(
        reviews.iter().filter(|r| r.role == "owner").count(),
        1,
        "nothing new"
    );

    // The re-run goes to the Mac: its answer, not "re-run it there".
    let (code, message) = error(call(&mut app, w::rerun(&b.win_app, &head, "rust")).await);
    assert_eq!(code, "not_found", "the Mac answered: {message}");
    let (code, _) = error(call(&mut token, w::rerun(&b.win_app, &head, "rust")).await);
    assert_eq!(code, "forbidden", "not on the owner token");
}
