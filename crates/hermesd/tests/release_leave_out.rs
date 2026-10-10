//! The owner's Leave out, PR-7 (H-272; UX-051 decision 10): left-out PRs
//! that merged last are cut away; earlier ones are reverted on main by an
//! owner-authored PR through the normal queue, then the release is cut
//! again; a later PR touching the same files is refused by name.

mod common;

use chrono::{Duration, Utc};
use common::cuts::{checkout, cut, cuts, get, leave_out, main_tip, merged, numbers, DEVOPS};
use common::WsClient;
use hermesd::board::release::leave_out_cli::{revert_in, Plan};
use hermesd::board::release::leave_out_gate::executor;
use hermesd::pr_cli::{merge_in, Plan as MergePlan};
use hermesd::prs::merge::answer;
use hermesd::prs::queue;
use serde_json::json;

fn items(release: &serde_json::Value) -> Vec<String> {
    release["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["item_id"].as_str().unwrap().to_string())
        .collect()
}

/// Left-out PRs that merged after every kept one: cut again before them,
/// owner device or ticket only, and the card stays where it is.
#[tokio::test]
async fn the_last_prs_are_cut_away() {
    let mut r = cuts().await;
    let (a, _, _) = merged(&mut r, "One", "H-1-one", "one.txt").await;
    let (_, _, two) = merged(&mut r, "Two", "H-2-two", "two.txt").await;
    let (c, three, _) = merged(&mut r, "Three", "H-3-three", "three.txt").await;
    let release = cut(&mut r, json!({"version": "0.18.0"})).await;
    let id = release["id"].as_str().unwrap().to_string();

    let ask = json!({"type": "release_leave_out", "project_id": r.project,
                     "release_id": id, "prs": [three]});
    let mut token = WsClient::connect_owner_token(&r.pair.d).await;
    assert_eq!(
        token.request(ask).await["type"],
        "error",
        "the owner token is refused"
    );

    let out = leave_out(&r, &id, &[three]).await;
    assert_eq!(out["result"]["mode"], "recut", "{out}");
    let after = get(&mut r, &id).await;
    assert_eq!(after["commit"], two.as_str(), "cut before #{three}");
    assert!(!items(&after).contains(&c), "{after}");
    assert!(items(&after).contains(&a));
    assert_eq!(
        r.column(&c),
        "verify",
        "it stays on main for the next release"
    );
}

/// An earlier PR is reverted on main by the owner's PR, which merges through
/// the queue with no bot review; then the release is cut again and the card
/// goes back to Doing.
#[tokio::test]
async fn an_earlier_pr_is_reverted_through_the_owners_pr() {
    let mut r = cuts().await;
    let (a, one, _) = merged(&mut r, "One", "H-1-one", "one.txt").await;
    let (b, _, _) = merged(&mut r, "Two", "H-2-two", "two.txt").await;
    let release = cut(&mut r, json!({"version": "0.18.0"})).await;
    let id = release["id"].as_str().unwrap().to_string();

    let out = leave_out(&r, &id, &[one]).await;
    assert_eq!(out["result"]["mode"], "revert", "{out}");
    let lo = out["result"]["leave_out"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let app = r.pair.d.app.clone();
    let devops = r.pair.ids[DEVOPS].clone();
    assert_eq!(
        app.db.open_tasks_for(&devops).unwrap().len(),
        1,
        "DevOps reverts"
    );

    // DevOps' command: the revert on a branch from main, pushed.
    let gate = executor(&app, &devops, "hermes/leave_out", &json!({"id": lo})).unwrap();
    let tree = checkout(&r, &r.origin, "revert");
    let head = revert_in(&tree, &Plan::from_gate(&gate).unwrap(), &r.dev).unwrap();
    let opened = executor(
        &app,
        &devops,
        "hermes/leave_out_pushed",
        &json!({"id": lo, "branch": gate["branch"], "head": head}),
    )
    .unwrap();
    let number = opened["number"].as_u64().unwrap() as u32;
    let pr = app
        .db
        .board_read(|t| t.pr(&r.project, number))
        .unwrap()
        .unwrap();
    assert!(
        pr.author.starts_with("owner:"),
        "the owner's PR: {}",
        pr.author
    );

    // The queue takes it with no review, and DevOps merges it.
    let now = Utc::now();
    queue::step(&app, now).unwrap();
    queue::step(&app, now + Duration::seconds(11)).unwrap();
    let check = answer(&app, &devops, "hermes/pr_merge", &json!({"number": number})).unwrap();
    let plan = MergePlan {
        head: check["head_sha"].as_str().unwrap().into(),
        branch: check["branch"].as_str().unwrap().into(),
        repo: check["repo"].as_str().unwrap().into(),
        repo_url: check["repo_url"].as_str().unwrap().into(),
        dry_run: false,
        stale: Vec::new(),
    };
    let done = merge_in(&checkout(&r, &r.origin, "merge"), &plan).unwrap();
    answer(
        &app,
        &devops,
        "hermes/pr_merged",
        &json!({"number": number, "sha": head,
                "branch": {"deleted": done.branch_deleted, "note": done.branch_note}}),
    )
    .unwrap();

    let after = get(&mut r, &id).await;
    assert_eq!(
        after["commit"],
        main_tip(&r.origin).as_str(),
        "cut at the new main"
    );
    assert_eq!(items(&after), std::slice::from_ref(&b), "{after}");
    let reverted = after["prs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["number"] == one)
        .unwrap()
        .clone();
    assert_eq!(reverted["reverted"], true, "{after}");
    assert_eq!(r.column(&a), "doing", "left out of 0.18.0");
    let original = r.bots[0].call("pr_get", json!({"number": one})).await;
    assert_eq!(original["pr"]["reverted_by"], number, "{original}");
}

/// A later PR that touched the same file can't be kept: refused by name.
#[tokio::test]
async fn a_later_pr_on_the_same_lines_is_refused_by_name() {
    let mut r = cuts().await;
    let (_, one, _) = merged(&mut r, "One", "H-1-one", "shared.txt").await;
    let (later, two, _) = merged(&mut r, "Two", "H-2-two", "shared.txt").await;
    merged(&mut r, "Three", "H-3-three", "other.txt").await;
    let release = cut(&mut r, json!({"version": "0.18.0"})).await;
    let id = release["id"].as_str().unwrap().to_string();
    let out = leave_out(&r, &id, &[one]).await;
    let why = out["message"].as_str().unwrap_or_default();
    assert!(why.contains(&format!("PR #{two} ({later})")), "{out}");
    assert!(why.contains("shared.txt"), "{out}");
    assert_eq!(
        numbers(&get(&mut r, &id).await["prs"]).len(),
        3,
        "nothing changed"
    );
}
