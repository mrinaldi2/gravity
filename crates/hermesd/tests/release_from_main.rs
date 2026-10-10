//! Releases cut from main, PR-7 (H-272; H-261 §6.1–6.3): `release_cut` takes
//! a commit on main (its tip by default) and computes what is in it from the
//! PRs merged since the last tag; `release tag` pushes desktop-v<version> at
//! that commit only after the owner's approval, with no release branch; and
//! the gate (builds, freeze, ruling, deploy, rollback) runs on such a
//! release end to end.

mod common;

use common::cuts::{checkout, cut, cuts, get, main_tip, merged, numbers, DEVOPS};
use common::prs::{clone, commit, error};
use common::releases::rule;
use common::repo::git;
use common::WsClient;
use hermesd::board::release::tag::answer;
use hermesd::board::release::tag_cli::{tag_in, Plan};
use serde_json::json;

/// AC1: the tip by default; a commit that isn't on main is refused.
#[tokio::test]
async fn a_cut_is_on_main_and_defaults_to_its_tip() {
    let mut r = cuts().await;
    let (a, one, _) = merged(&mut r, "One", "H-1-one", "one.txt").await;
    let (b, two, tip) = merged(&mut r, "Two", "H-2-two", "two.txt").await;

    // A commit pushed to a branch only is not on main.
    let side = r.dev.join("side");
    clone(&r.origin, &side, "side");
    let off_main = commit(&side, "side.txt", "x\n");
    git(&side, &["push", "-q", "origin", "side"]);
    let raw = r.bots[DEVOPS]
        .call_raw(
            "release_cut",
            json!({"version": "0.18.0", "commit": off_main}),
        )
        .await;
    assert!(error(&raw).contains("isn't on main"), "{raw}");
    let raw = r.bots[1]
        .call_raw("release_cut", json!({"version": "0.18.0"}))
        .await;
    assert!(error(&raw).contains("role"), "only DevOps cuts: {raw}");

    // An earlier commit on main cuts there, with only what it holds.
    let first = r
        .pair
        .d
        .app
        .db
        .board_read(|t| t.pr(&r.project, one))
        .unwrap()
        .unwrap();
    let earlier = cut(
        &mut r,
        json!({"version": "0.17.9", "commit": first.merged_sha.unwrap()}),
    )
    .await;
    assert_eq!(numbers(&earlier["prs"]), [one as u64]);
    r.bots[DEVOPS]
        .call(
            "release_cancel",
            json!({"release_id": earlier["id"], "reason": "test"}),
        )
        .await;

    let release = cut(&mut r, json!({"version": "0.18.0"})).await;
    assert_eq!(release["commit"], tip.as_str(), "the tip: {release}");
    assert_eq!(release["tag"], "", "not tagged yet");
    assert_eq!(release["tag_name"], "desktop-v0.18.0");
    assert_eq!(numbers(&release["prs"]), [one as u64, two as u64]);
    let items: Vec<&str> = release["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["item_id"].as_str().unwrap())
        .collect();
    assert_eq!(items.len(), 2);
    assert!(items.contains(&a.as_str()) && items.contains(&b.as_str()));
}

/// AC2: since the previous tag only; planned cards not merged show as not
/// merged yet, and merged cards nobody planned as also included.
#[tokio::test]
async fn a_cut_lists_what_merged_since_the_last_tag() {
    let mut r = cuts().await;
    merged(&mut r, "Old", "H-1-old", "old.txt").await;
    let old = cut(&mut r, json!({"version": "0.17.9"})).await;
    let old_id = old["id"].as_str().unwrap().to_string();
    r.pair
        .d
        .app
        .db
        .board_tx(|t| t.set_release_tag(&old_id, "desktop-v0.17.9"))
        .unwrap();

    let planned_card = r.card("Planned", "doing");
    let unmerged = r.card("Not yet", "doing");
    let plan = r.bots[0]
        .call(
            "release_plan",
            json!({"name": "0.18.0", "items": [planned_card, unmerged]}),
        )
        .await["release"]
        .clone();
    let plan_id = plan["id"].as_str().unwrap().to_string();
    // The planned card's PR, and a card nobody planned.
    let tree = r.dev.join("gravity-wt-desktopdev-planned");
    clone(&r.origin, &tree, "H-3-planned");
    let head = commit(&tree, "planned.txt", "p\n");
    git(&tree, &["push", "-q", "origin", "H-3-planned"]);
    let pr = r.bots[1]
        .call(
            "pr_open",
            json!({"item": planned_card, "branch": "H-3-planned"}),
        )
        .await["pr"]["number"]
        .as_u64()
        .unwrap();
    git(&tree, &["push", "-q", "origin", "HEAD:main"]);
    let devops = r.pair.ids[DEVOPS].clone();
    r.pair
        .d
        .app
        .db
        .board_tx(|t| {
            let p = t.pr(&r.project, pr as u32)?.unwrap();
            t.set_pr_merged(&p, &head, &devops)
        })
        .unwrap();
    common::cuts::to_column(&r, &planned_card, "verify");
    let (extra, extra_pr, _) = merged(&mut r, "Extra", "H-4-extra", "extra.txt").await;

    let release = cut(&mut r, json!({"version": "0.18.0", "release_id": plan_id})).await;
    assert_eq!(release["status"], "assembling", "{release}");
    assert_eq!(numbers(&release["prs"]), [pr], "planned and merged");
    assert_eq!(numbers(&release["also_included"]), [extra_pr as u64]);
    assert_eq!(release["also_included"][0]["item_id"], extra.as_str());
    assert_eq!(release["not_merged"], json!([unmerged]));
    assert_eq!(
        release["previous_commit"], old["commit"],
        "from the last tag"
    );
}

/// AC3 and AC5: on a release cut from main, builds come from the cut,
/// the gate runs as before, the tag goes on only after the owner's approval
/// (no release branch), and deploy and rollback still work.
#[tokio::test]
async fn a_cut_release_goes_through_the_gate_and_is_tagged_after_approval() {
    let mut r = cuts().await;
    let (item, _, _) = merged(&mut r, "Ship", "H-1-ship", "ship.txt").await;
    let release = cut(&mut r, json!({"version": "0.18.0"})).await;
    let id = release["id"].as_str().unwrap().to_string();
    let commit = release["commit"].as_str().unwrap().to_string();
    let build = |source: &str| {
        json!({"release_id": id, "platform": "daemon", "version": "0.18.0",
               "artifact": "/builds/hermesd", "sha256": "a".repeat(64),
               "source_commit": source})
    };
    let raw = r.bots[DEVOPS]
        .call_raw("release_attach_build", build(&"b".repeat(40)))
        .await;
    assert!(error(&raw).contains("was cut at"), "{raw}");
    r.bots[DEVOPS]
        .call("release_attach_build", build(&commit))
        .await;
    r.bots[2]
        .call(
            "release_test",
            json!({"release_id": id, "machine": "mac", "build_sha256": "a".repeat(64),
                   "result": "pass"}),
        )
        .await;
    let submitted = r.bots[DEVOPS]
        .call("release_submit", json!({"release_id": id}))
        .await["release"]
        .clone();
    assert_eq!(submitted["status"], "awaiting_owner", "{submitted}");

    let app = r.pair.d.app.clone();
    let devops = r.pair.ids[DEVOPS].clone();
    let gate = |method: &str, params| answer(&app, &devops, method, &params);
    let early = gate("hermes/release_tag", json!({"release_id": id})).unwrap_err();
    assert!(
        early
            .to_string()
            .contains("only a release the owner approved"),
        "{early}"
    );
    let land = hermesd::board::release::gates::serve(
        &app,
        &devops,
        &json!({"method": "hermes/release_land", "params": {"release_id": id}}),
    )
    .unwrap();
    assert!(
        land["error"]["message"]
            .as_str()
            .unwrap()
            .contains("release tag"),
        "{land}"
    );

    let mut owner = WsClient::connect(&r.pair.d).await;
    let ruled = rule(
        &mut owner,
        &submitted,
        json!([{"item_id": item, "verdict": "ship"}]),
    )
    .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    let plan = gate("hermes/release_tag", json!({"release_id": id})).unwrap();
    assert_eq!(plan["tag"], "desktop-v0.18.0");
    let tree = checkout(&r, &r.origin, "tag");
    tag_in(&tree, &Plan::from_gate(&plan, &id, false).unwrap()).unwrap();
    let tagged = git(&r.origin, &["rev-parse", "refs/tags/desktop-v0.18.0^{}"]);
    assert_eq!(tagged.trim(), commit, "the tag is on the cut commit");
    gate(
        "hermes/release_tagged",
        json!({"release_id": id, "tag": "desktop-v0.18.0", "commit": commit}),
    )
    .unwrap();
    assert_eq!(get(&mut r, &id).await["tag"], "desktop-v0.18.0");
    let heads = git(
        &r.origin,
        &["for-each-ref", "--format=%(refname)", "refs/heads"],
    );
    assert!(!heads.contains("release/"), "no release branch: {heads}");
    assert_eq!(main_tip(&r.origin), commit, "main didn't move");

    // Deploy, a failed install, and the rollback, as for any release.
    let at_mac = json!({"release_id": id, "machine": "mac"});
    let deploying = r.bots[DEVOPS].call("release_deploy", at_mac.clone()).await;
    assert_eq!(deploying["release"]["status"], "deploying");
    let failed = r.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "mac", "result": "failed", "smoke": "fail"}),
        )
        .await;
    assert_eq!(failed["release"]["status"], "partially_deployed");
    r.bots[DEVOPS].call("release_rollback", at_mac).await;
    let back = r.bots[2]
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "mac", "result": "rolled_back"}),
        )
        .await;
    assert_eq!(back["release"]["status"], "rolled_back", "{back}");
}
