//! The PR read surface over WS, PR-8 (H-273; H-261 §9): `pr_list`, `pr_get`,
//! `pr_diff` and `pr_comments` as `hermes.pr.v1` binary frames on the
//! board's home; the pushes every PR change sends; the owner's `check_rerun`;
//! and a linked computer's check log kept as opaque text.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use bus::contract::pr::{self as p, pr_push::Push, pr_response::Response};
use common::pr_wire::{self as w, call, error, expect_push, response};
use common::prs::{approve, commit, opened, report, setup, Repo};
use common::repo::git;
use common::WsClient;
use hermesd::prs::check_model::{CheckResult, NewCheck, Report};
use prost::Message;
use serde_json::json;

fn updated(r: &Repo) -> Push {
    Push::PrUpdated(p::PrUpdated {
        project_id: r.project.clone(),
        number: 1,
    })
}

/// A finished check `name` on `sha`, as a run that couldn't start leaves it.
fn errored_check(r: &Repo, sha: &str, name: &str) {
    let db = &r.pair.d.app.db;
    let pr = db.board_read(|t| t.pr(&r.project, 1)).unwrap().unwrap();
    let check = NewCheck {
        name: name.into(),
        run: "true".into(),
        needs: Vec::new(),
        machine: None,
        required: true,
        result: CheckResult::Error,
        note: Some("couldn't start".into()),
    };
    db.board_tx(|t| t.queue_checks(&r.project, &pr.repo, sha, "tree", &[check]))
        .unwrap();
}

/// AC1, AC2: the list, one PR, its comments, and its diff for head, for a
/// range (the `approved..head` delta) and for one path, cut at 2 MB.
#[tokio::test]
async fn the_app_reads_prs_diffs_and_comments_over_binary_frames() {
    let mut r = setup().await;
    let (tree, first) = opened(&mut r).await;
    commit(&tree, "notes.txt", "one\ntwo\nthree\nfour\nfive\n");
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let head = report(&mut r, &tree).await;
    let mut app = WsClient::connect(&r.pair.d).await;

    let hello = common::raw_hello(&r.pair.d, &r.pair.d.app.owner.mint()).await;
    for cap in ["pull_requests", "checks"] {
        assert!(hello["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!(cap)));
    }
    assert_eq!(hello["contracts"]["pr"], 1, "{hello}");

    let open = w::list(&mut app, &r.project, &[]).await;
    assert_eq!(open.len(), 1);
    let listed = &open[0];
    assert_eq!((listed.number, listed.state), (1, p::PrState::Open as i32));
    assert_eq!(listed.author.as_ref().unwrap().name, "Desktop Dev");
    assert_eq!(listed.item_title, "Search");
    assert!(w::list(&mut app, &r.project, &[p::PrState::Closed])
        .await
        .is_empty());

    let pr = w::pr(&mut app, &r.project, 1).await;
    assert_eq!(pr.head_sha, head);
    assert_eq!(pr.required_roles, ["architect"]);
    let mergeable = pr.mergeable.expect("mergeable");
    assert!(!mergeable.ok && !mergeable.blockers.is_empty());
    let (code, _) = error(call(&mut app, w::get(&r.project, 9)).await);
    assert_eq!(code, "not_found");

    // Head: every file of the change since main, with its counts.
    let Response::PrDiff(all) =
        response(call(&mut app, w::diff(&r.project, None, None, None)).await)
    else {
        panic!("a diff");
    };
    assert_eq!(all.to_sha, head);
    let counts: Vec<(String, u32, u32)> = all
        .files
        .iter()
        .map(|f| (f.path.clone(), f.additions, f.deletions))
        .collect();
    assert_eq!(counts, [("a.txt".into(), 1, 0), ("notes.txt".into(), 5, 0)]);
    assert!(
        all.diff.contains("+three") && !all.truncated,
        "{}",
        all.diff
    );

    // The delta since an approval on the first head.
    let Response::PrDiff(delta) =
        response(call(&mut app, w::diff(&r.project, Some(&first), None, None)).await)
    else {
        panic!("a diff");
    };
    assert_eq!(delta.from_sha, first);
    let paths: Vec<&str> = delta.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["notes.txt"]);
    let Response::PrDiff(one) =
        response(call(&mut app, w::diff(&r.project, None, None, Some("a.txt"))).await)
    else {
        panic!("a diff");
    };
    assert_eq!(one.files.len(), 1);
    assert!(!one.diff.contains("notes.txt"));

    // Only commit ids reach git.
    for bad in ["--output=/tmp/x", "HEAD~1", "deadbeefdeadbeef"] {
        let (code, _) = error(call(&mut app, w::diff(&r.project, Some(bad), None, None)).await);
        assert_eq!(code, "invalid_request", "{bad}");
    }
    let (code, _) = error(call(&mut app, w::diff(&r.project, None, None, Some(":(top)x"))).await);
    assert_eq!(code, "invalid_request");

    // Past 2 MB the text stops on a whole line; the file list stays whole.
    let line = "x".repeat(99);
    let big = format!("{line}\n").repeat(30_000);
    commit(&tree, "big.txt", &big);
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    report(&mut r, &tree).await;
    let Response::PrDiff(cut) =
        response(call(&mut app, w::diff(&r.project, None, None, None)).await)
    else {
        panic!("a diff");
    };
    assert!(cut.truncated);
    assert!(cut.diff.len() <= 2 * 1024 * 1024 && cut.diff.ends_with('\n'));
    let big_file = cut.files.iter().find(|f| f.path == "big.txt").unwrap();
    assert_eq!(big_file.additions, 30_000);

    // Comments as shown on head, with who wrote them.
    let head = common::prs::head(&tree);
    r.bots[2]
        .call(
            "pr_comment",
            json!({"number": 1, "sha": head, "path": "notes.txt", "line": 2, "body": "Two?"}),
        )
        .await;
    let request = p::pr_request::Request::PrComments(p::PrCommentsRequest {
        project_id: r.project.clone(),
        number: 1,
        sha: None,
    });
    let Response::PrComments(shown) = response(call(&mut app, request).await) else {
        panic!("comments");
    };
    assert_eq!(shown.sha, head);
    let [c] = &shown.comments[..] else {
        panic!("one comment: {shown:?}");
    };
    assert_eq!(
        (c.line, c.side, c.body.as_str()),
        (2, p::Side::New as i32, "Two?")
    );
    let Some(p::reviewer::Who::Bot(by)) = c.author.clone().unwrap().who else {
        panic!("a bot");
    };
    assert_eq!(by.name, "Architect");
}

/// AC3: a PR request starts the project's pushes; a comment, a review, a
/// check and the merge queue each push as they commit.
#[tokio::test]
async fn every_pr_change_is_pushed() {
    let mut r = setup().await;
    let (_tree, head) = opened(&mut r).await;
    let mut app = WsClient::connect(&r.pair.d).await;
    let mut idle = WsClient::connect(&r.pair.d).await;
    w::list(&mut app, &r.project, &[]).await;

    r.bots[2]
        .call(
            "pr_comment",
            json!({"number": 1, "sha": head, "path": "a.txt", "line": 1, "body": "Hm."}),
        )
        .await;
    expect_push(&mut app, updated(&r)).await;
    approve(&mut r, &head).await;
    expect_push(&mut app, updated(&r)).await;

    errored_check(&r, &head, "rust");
    expect_push(
        &mut app,
        Push::CheckUpdated(p::CheckUpdated {
            project_id: r.project.clone(),
            sha: head.clone(),
            name: "rust".into(),
        }),
    )
    .await;
    expect_push(&mut app, updated(&r)).await;

    let db = &r.pair.d.app.db;
    let pr = db.board_read(|t| t.pr(&r.project, 1)).unwrap().unwrap();
    db.board_tx(|t| t.enqueue(&pr, chrono::Utc::now())).unwrap();
    expect_push(
        &mut app,
        Push::MergeQueueChanged(p::MergeQueueChanged {
            project_id: r.project.clone(),
        }),
    )
    .await;

    // A connection that never asked about PRs gets none of it.
    assert_eq!(
        w::next_push(&mut idle, Duration::from_millis(500)).await,
        None
    );
}

/// The owner re-runs a check from the app or a device, never on the owner
/// token or a read-only grant; and a linked computer's log stays text.
#[tokio::test]
async fn the_owner_reruns_a_check_and_a_remote_log_stays_text() {
    let mut r = setup().await;
    let (_tree, head) = opened(&mut r).await;
    errored_check(&r, &head, "rust");

    let mut token = WsClient::connect_owner_token(&r.pair.d).await;
    let (code, _) = error(call(&mut token, w::rerun(&r.project, &head, "rust")).await);
    assert_eq!(code, "forbidden");
    let mut app = WsClient::connect(&r.pair.d).await;
    let device = app
        .request(json!({"type": "create_device", "name": "viewer", "capabilities": ["read"]}))
        .await;
    let mut viewer = WsClient::connect_as(&r.pair.d, common::token_str(&device)).await;
    let (code, _) = error(call(&mut viewer, w::rerun(&r.project, &head, "rust")).await);
    assert_eq!(code, "forbidden");
    assert_eq!(w::list(&mut viewer, &r.project, &[]).await.len(), 1);

    let mut pushes = Vec::new();
    let body = w::call_keeping(&mut app, w::rerun(&r.project, &head, "rust"), &mut pushes).await;
    let Response::Check(check) = response(body) else {
        panic!("a check");
    };
    assert_eq!(check.result, p::CheckResult::Queued as i32);
    expect_push(
        &mut app,
        Push::CheckUpdated(p::CheckUpdated {
            project_id: r.project.clone(),
            sha: head.clone(),
            name: "rust".into(),
        }),
    )
    .await;
    let (code, _) = error(call(&mut app, w::rerun(&r.project, &head, "rust")).await);
    assert_eq!(code, "conflict", "a queued check isn't re-run");

    // A log on the PC is `<computer>:<path>`: shown as it is, never opened
    // here, even where this disk has a file at that path.
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("check.log");
    std::fs::write(&local, "SECRET on this disk\n").unwrap();
    let log = format!("win:{}", local.display());
    let db = &r.pair.d.app.db;
    let run = db
        .board_read(|t| t.check_run(&r.project, &head, "rust"))
        .unwrap()
        .unwrap();
    db.board_tx(|t| {
        t.report_check(
            &run.id,
            &Report {
                result: CheckResult::Fail,
                ran_on: "win",
                log_artifact: Some(&log),
                tool_versions: &BTreeMap::new(),
            },
        )
    })
    .unwrap();
    let body = call(&mut app, w::get(&r.project, 1)).await;
    let bytes = match &body {
        bus::contract::wire::envelope::Body::PrResponse(r) => r.encode_to_vec(),
        other => panic!("a PR: {other:?}"),
    };
    let Response::Pr(pr) = response(body) else {
        panic!("a PR");
    };
    assert_eq!(pr.checks[0].log_url, log);
    assert_eq!(pr.checks[0].ran_on, "win");
    assert!(!String::from_utf8_lossy(&bytes).contains("SECRET"));
}
