//! Line comments, PR-3b (H-282; H-261 §1.4, §5.1(3)): a comment is anchored
//! to the commit it was written on and shown where its line went on a newer
//! head, or outdated when the line is gone, never on a wrong line; replies
//! join the thread, resolving closes it, and an open `must` thread keeps the
//! PR from merging.

mod common;

use common::prs::{commit, error, opened, pr, report, setup, Repo};
use common::repo::git;
use common::WsClient;
use serde_json::{json, Value};

const NOTES: &str = "one\ntwo\nthree\nfour\nfive\n";

async fn comments(r: &mut Repo) -> Vec<Value> {
    r.bots[0].call("pr_comments", json!({"number": 1})).await["comments"]
        .as_array()
        .unwrap()
        .clone()
}

fn by_id<'a>(all: &'a [Value], id: &Value) -> &'a Value {
    all.iter().find(|c| c["id"] == *id).unwrap()
}

/// AC1 and AC2: shown where its line went on a newer head; outdated when the
/// line was changed, never moved onto another line.
#[tokio::test]
async fn a_comment_follows_its_line_or_says_it_is_outdated() {
    let mut r = setup().await;
    let (tree, _) = opened(&mut r).await;
    commit(&tree, "notes.txt", NOTES);
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let first = report(&mut r, &tree).await;
    let on_four = r.bots[2]
        .call(
            "pr_comment",
            json!({"number": 1, "sha": first, "path": "notes.txt", "line": 4, "body": "Why four?"}),
        )
        .await["comment"]["id"]
        .clone();
    let on_two = r.bots[2]
        .call(
            "pr_comment",
            json!({"number": 1, "sha": first, "path": "notes.txt", "line": 2, "body": "Fine."}),
        )
        .await["comment"]["id"]
        .clone();

    // Two lines go in at the top: both comments move down by two.
    commit(&tree, "notes.txt", &format!("zero\nhalf\n{NOTES}"));
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    let second = report(&mut r, &tree).await;
    let all = comments(&mut r).await;
    let four = by_id(&all, &on_four);
    assert_eq!(four["shown_on"], second.as_str());
    assert_eq!(four["shown_line"], 6, "{four}");
    assert_eq!(four["outdated"], false);
    assert_eq!(four["line"], 4, "the anchor itself never changes");
    assert_eq!(by_id(&all, &on_two)["shown_line"], 4);

    // "four" is rewritten: its comment is outdated, not shown on a neighbour.
    commit(
        &tree,
        "notes.txt",
        "zero\nhalf\none\ntwo\nthree\nFOUR\nfive\n",
    );
    git(&tree, &["push", "-q", "origin", "H-1-search"]);
    report(&mut r, &tree).await;
    let all = comments(&mut r).await;
    let four = by_id(&all, &on_four);
    assert_eq!(four["outdated"], true, "{four}");
    assert_eq!(four["shown_line"], Value::Null);
    assert_eq!(
        by_id(&all, &on_two)["shown_line"],
        4,
        "untouched lines keep their place"
    );

    // Asked on the commit it was written on, it is where it was.
    let then = r.bots[0]
        .call("pr_comments", json!({"number": 1, "sha": first}))
        .await["comments"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(by_id(&then, &on_four)["shown_line"], 4);

    // A comment on a commit the PR never had is refused.
    let stranger = r.bots[2]
        .call_raw(
            "pr_comment",
            json!({"number": 1, "sha": "0".repeat(40), "path": "notes.txt", "line": 1,
                   "body": "?"}),
        )
        .await;
    assert!(
        error(&stranger).contains("isn't a commit of PR #1"),
        "{stranger}"
    );
}

/// AC3: replies join the thread; an open `must` thread blocks the merge
/// until its author, the PR's author or the owner resolves it.
#[tokio::test]
async fn a_must_thread_blocks_until_resolved() {
    let mut r = setup().await;
    let (_, head) = opened(&mut r).await;
    let must = r.bots[2]
        .call(
            "pr_comment",
            json!({"number": 1, "sha": head, "path": "a.txt", "line": 1,
                   "body": "Rename this.", "severity": "must"}),
        )
        .await["comment"]["id"]
        .clone();
    let reply = r.bots[1]
        .call(
            "pr_comment",
            json!({"number": 1, "sha": head, "path": "x", "line": 9, "body": "Done.",
                   "reply_to": must}),
        )
        .await["comment"]
        .clone();
    assert_eq!(reply["reply_to"], must);
    assert_eq!(reply["path"], "a.txt", "a reply takes its thread's anchor");
    let blocked = pr(&mut r).await;
    let blockers = blocked["mergeable"]["blockers"].as_array().unwrap();
    assert!(
        blockers
            .iter()
            .any(|b| b["kind"] == "unresolved_must" && b["subject"] == "comments"),
        "{blocked}"
    );

    let stranger = r.bots[0]
        .call_raw(
            "pr_comment_resolve",
            json!({"number": 1, "comment_id": must}),
        )
        .await;
    assert!(error(&stranger).contains("resolves it"), "{stranger}");
    // Resolving through a reply resolves the thread.
    let resolved = r.bots[2]
        .call(
            "pr_comment_resolve",
            json!({"number": 1, "comment_id": reply["id"]}),
        )
        .await["comment"]
        .clone();
    assert_eq!(resolved["id"], must);
    assert_eq!(resolved["resolved"], true);
    let open = pr(&mut r).await;
    let blockers = open["mergeable"]["blockers"].as_array().unwrap();
    assert!(
        blockers.iter().all(|b| b["subject"] != "comments"),
        "{open}"
    );

    // The owner resolves any thread, from the app; never with the owner token.
    let second = r.bots[2]
        .call(
            "pr_comment",
            json!({"number": 1, "sha": head, "path": "a.txt", "line": 1,
                   "body": "And this.", "severity": "must"}),
        )
        .await["comment"]["id"]
        .clone();
    let resolve = json!({"type": "pr_comment_resolve", "project_id": r.project,
                         "number": 1, "comment_id": second});
    let mut token = WsClient::connect_owner_token(&r.pair.d).await;
    assert_eq!(token.request(resolve.clone()).await["type"], "error");
    let mut app = WsClient::connect(&r.pair.d).await;
    let done = app.request(resolve).await;
    assert_eq!(done["comment"]["resolved"], true, "{done}");
}
