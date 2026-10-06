//! Closing an approved package that a later deployed release contains
//! (H-121): a chain of approved, never-installed packages closes when a
//! release built on top of them deploys; nothing else does.

mod common;

use std::process::Command;

use common::deployed_via::{approved, deploy, git, history, via};
use common::releases::releases;
use common::tasks::error_text;
use common::WsClient;
use serde_json::json;

#[tokio::test]
async fn a_chain_of_approved_packages_closes_through_the_deployed_one() {
    let mut r = releases(4).await;
    let repo = tempfile::tempdir().unwrap();
    let [a, b, c, d] = history(repo.path());
    r.pair
        .d
        .app
        .db
        .set_project_repo(
            &r.project,
            Some(&bus::ProjectRepo {
                url: repo.path().display().to_string(),
                branch: "main".into(),
            }),
        )
        .unwrap();
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    let p2 = approved(&mut r, &mut owner, "0.16.2", &items[0], Some(&a)).await;
    let p3 = approved(&mut r, &mut owner, "0.16.3", &items[1], Some(&b)).await;
    let side = approved(&mut r, &mut owner, "side", &items[3], Some(&d)).await;
    let p4 = approved(&mut r, &mut owner, "0.16.4", &items[2], Some(&c)).await;

    // Not until the later one is deployed.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&p3, &p4))
        .await;
    assert!(error_text(&raw).contains("not deployed"), "{raw}");
    let raw = r.bots[2]
        .call_raw("release_deployed_via", via(&p3, &p4))
        .await;
    assert!(error_text(&raw).contains("role"), "{raw}");
    deploy(&mut r, p4["id"].as_str().unwrap()).await;

    // 0.16.3 (B) and, through it or directly, 0.16.2 (A) close; D doesn't.
    let closed = r.bots[1].call("release_deployed_via", via(&p3, &p4)).await["release"].clone();
    assert_eq!(closed["status"], "deployed", "{closed}");
    assert_eq!(
        closed["deployments"],
        json!([]),
        "no deployment is invented"
    );
    let event = closed["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "deployed_via")
        .cloned()
        .expect("event");
    assert_eq!(event["note"], "deployed via 0.16.4");
    assert_eq!(event["detail"]["basis"], "ancestry");
    assert_eq!(r.column(&items[1]), "done");
    let closed = r.bots[1].call("release_deployed_via", via(&p2, &p3)).await["release"].clone();
    assert_eq!(closed["status"], "deployed", "{closed}");
    assert_eq!(r.column(&items[0]), "done");

    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&side, &p4))
        .await;
    assert!(error_text(&raw).contains("doesn't contain"), "{raw}");
    assert_eq!(r.column(&items[3]), "deploying", "left where it was");
    // Closed once is closed.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&p2, &p4))
        .await;
    assert!(
        error_text(&raw).contains("only an approved package"),
        "{raw}"
    );
}

/// CE-015 M1: a package from before source commits were recorded closes
/// only when its release branch (or tag) is in the via commit's history;
/// what it built proves nothing.
#[tokio::test]
async fn a_package_without_a_commit_closes_only_through_its_release_branch() {
    let mut r = releases(6).await;
    let repo = tempfile::tempdir().unwrap();
    let [a, _b, c, d] = history(repo.path());
    git(repo.path(), &["branch", "release/desktop-0.16.2", &a]);
    git(repo.path(), &["tag", "desktop-v0.16.3", &a]);
    git(repo.path(), &["branch", "release/desktop-0.15.9", &d]);
    r.pair
        .d
        .app
        .db
        .set_project_repo(
            &r.project,
            Some(&bus::ProjectRepo {
                url: repo.path().display().to_string(),
                branch: "main".into(),
            }),
        )
        .unwrap();
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    let old = approved(&mut r, &mut owner, "0.16.2", &items[0], None).await;
    let tagged = approved(&mut r, &mut owner, "0.16.3", &items[1], None).await;
    let side = approved(&mut r, &mut owner, "0.15.9", &items[2], None).await;
    let unbranched = approved(&mut r, &mut owner, "0.14.0", &items[3], None).await;
    let open = approved(&mut r, &mut owner, "0.16.2-b", &items[4], None).await;
    let newer = approved(&mut r, &mut owner, "0.16.4", &items[5], Some(&c)).await;
    deploy(&mut r, newer["id"].as_str().unwrap()).await;

    // (b) Its branch isn't in the via commit's history: refused, left put.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&side, &newer))
        .await;
    assert!(error_text(&raw).contains("doesn't contain"), "{raw}");
    assert_eq!(r.column(&items[2]), "deploying", "left where it was");
    // (c) No branch or tag at all: refused.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&unbranched, &newer))
        .await;
    assert!(
        error_text(&raw).contains("no release branch or tag"),
        "{raw}"
    );
    // A deployment still open on it: refused.
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": open["id"], "machine": "mac"}),
        )
        .await;
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&open, &newer))
        .await;
    assert!(error_text(&raw).contains("is deploying"), "{raw}");

    // Its post-install criteria first (H-116).
    let db = &r.pair.d.app.db;
    let item = db.get_item(&items[0]).unwrap().unwrap();
    let texts = vec!["survives a reboot".to_string()];
    let edit = hermesd::db::ItemEdit {
        acceptance_criteria: Some(&texts),
        ..hermesd::db::ItemEdit::default()
    };
    let hermesd::db::Write::Done(item) = db
        .update_item(&item.id, item.version, &edit, &hermesd::actor::Actor::User)
        .unwrap()
    else {
        unreachable!()
    };
    db.board_tx(|t| {
        t.flag_ac(
            &item.id,
            item.version,
            0,
            true,
            &hermesd::actor::Actor::User,
        )
    })
    .unwrap();
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&old, &newer))
        .await;
    assert!(error_text(&raw).contains("post-install"), "{raw}");
    let item = db.get_item(&items[0]).unwrap().unwrap();
    db.check_ac(
        &item.id,
        item.version,
        0,
        true,
        None,
        &hermesd::actor::Actor::User,
    )
    .unwrap();

    // (a) Its release branch, and a release tag, are in the via history.
    for (package, reference) in [
        (&old, "refs/heads/release/desktop-0.16.2"),
        (&tagged, "refs/tags/desktop-v0.16.3"),
    ] {
        let closed = r.bots[1]
            .call("release_deployed_via", via(package, &newer))
            .await["release"]
            .clone();
        assert_eq!(closed["status"], "deployed", "{closed}");
        let detail = closed["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["kind"] == "deployed_via")
            .map(|e| e["detail"].clone())
            .expect("event");
        assert_eq!(detail["basis"], "release_branch", "{detail}");
        assert_eq!(detail["reference"], reference, "{detail}");
        assert_eq!(detail["commit"], a.as_str());
    }
    assert_eq!(r.column(&items[0]), "done");
    assert_eq!(r.column(&items[1]), "done");
}

/// H-146: a via package whose builds record no source commit is read from
/// its own release branch, else its tag; with neither, it is refused.
#[tokio::test]
async fn a_via_package_without_a_commit_is_read_from_its_release_branch_or_tag() {
    let mut r = releases(6).await;
    let repo = tempfile::tempdir().unwrap();
    let [a, _b, c, d] = history(repo.path());
    // The branch wins over a tag that points elsewhere.
    git(repo.path(), &["branch", "release/desktop-0.16.4", &c]);
    git(repo.path(), &["tag", "desktop-v0.16.4", &d]);
    git(repo.path(), &["tag", "desktop-v0.16.5", &c]);
    r.pair
        .d
        .app
        .db
        .set_project_repo(
            &r.project,
            Some(&bus::ProjectRepo {
                url: repo.path().display().to_string(),
                branch: "main".into(),
            }),
        )
        .unwrap();
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    let branched_old = approved(&mut r, &mut owner, "0.16.2", &items[0], Some(&a)).await;
    let tagged_old = approved(&mut r, &mut owner, "0.16.3", &items[1], Some(&a)).await;
    let bare_old = approved(&mut r, &mut owner, "0.16.2-r2", &items[2], Some(&a)).await;
    let branched = approved(&mut r, &mut owner, "0.16.4", &items[3], None).await;
    let tagged = approved(&mut r, &mut owner, "0.16.5", &items[4], None).await;
    let bare = approved(&mut r, &mut owner, "0.16.6", &items[5], None).await;
    for v in [&branched, &tagged, &bare] {
        deploy(&mut r, v["id"].as_str().unwrap()).await;
    }

    // Neither a branch nor a tag for the via package: refused, left put.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&bare_old, &bare))
        .await;
    let text = error_text(&raw);
    assert!(
        text.contains("0.16.6 records no source commit and has no release branch or tag"),
        "{raw}"
    );
    assert_eq!(r.column(&items[2]), "deploying", "left where it was");

    for (old, via_package, reference, item) in [
        (
            &branched_old,
            &branched,
            "refs/heads/release/desktop-0.16.4",
            &items[0],
        ),
        (&tagged_old, &tagged, "refs/tags/desktop-v0.16.5", &items[1]),
    ] {
        let closed = r.bots[1]
            .call("release_deployed_via", via(old, via_package))
            .await["release"]
            .clone();
        assert_eq!(closed["status"], "deployed", "{closed}");
        let detail = closed["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["kind"] == "deployed_via")
            .map(|e| e["detail"].clone())
            .expect("event");
        assert_eq!(detail["basis"], "ancestry", "{detail}");
        assert_eq!(detail["via_reference"], reference, "{detail}");
        assert_eq!(detail["via_commit"], c.as_str(), "{detail}");
        assert_eq!(detail["commit"], a.as_str(), "{detail}");
        assert!(detail["via_commit_at"].is_string(), "{detail}");
        assert!(detail["via_submitted_at"].is_string(), "{detail}");
        assert_eq!(r.column(item), "done");
    }
}

/// CE-018: a release branch moves after its package ships (a respin's fixes
/// land on it), so a via package must be later than the old one (M1), and a
/// via commit read from a ref must be no newer than the via's submit (M2).
#[tokio::test]
async fn a_via_branch_that_moved_after_its_package_is_refused() {
    let mut r = releases(3).await;
    let repo = tempfile::tempdir().unwrap();
    let [a, _b, c, _d] = history(repo.path());
    git(repo.path(), &["branch", "release/desktop-0.16.4", &c]);
    r.pair
        .d
        .app
        .db
        .set_project_repo(
            &r.project,
            Some(&bus::ProjectRepo {
                url: repo.path().display().to_string(),
                branch: "main".into(),
            }),
        )
        .unwrap();
    let mut owner = WsClient::connect(&r.pair.d).await;
    let items = r.items.clone();
    let old = approved(&mut r, &mut owner, "0.16.3", &items[0], Some(&a)).await;
    let shipped = approved(&mut r, &mut owner, "0.16.4", &items[1], None).await;
    deploy(&mut r, shipped["id"].as_str().unwrap()).await;

    // A fix F lands on the shipped package's branch, after its submit, and
    // a respin is built from it but never installed.
    git(repo.path(), &["checkout", "-q", "release/desktop-0.16.4"]);
    let later = format!("@{} +0000", chrono::Utc::now().timestamp() + 3600);
    let out = Command::new("git")
        .current_dir(repo.path())
        .env("GIT_COMMITTER_DATE", &later)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["commit", "-q", "--allow-empty", "-m", "F"])
        .output()
        .expect("git");
    assert!(out.status.success(), "{out:?}");
    let f = git(repo.path(), &["rev-parse", "HEAD"]);
    git(repo.path(), &["checkout", "-q", "main"]);
    let respin = approved(&mut r, &mut owner, "0.16.4-r2", &items[2], Some(&f)).await;

    // (c) The respin, though the branch tip now holds F: refused (M1).
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&respin, &shipped))
        .await;
    assert!(
        error_text(&raw).contains("0.16.4 isn't later than 0.16.4-r2"),
        "{raw}"
    );
    assert_eq!(r.column(&items[2]), "deploying", "left where it was");

    // (a) The branch is newer than the via's submit: refused (M2), even
    // though the old commit is in its history.
    let raw = r.bots[1]
        .call_raw("release_deployed_via", via(&old, &shipped))
        .await;
    assert!(
        error_text(&raw).contains("release/desktop-0.16.4 has moved since 0.16.4 was submitted"),
        "{raw}"
    );
    assert_eq!(r.column(&items[0]), "deploying", "left where it was");

    // (b) Back where it was at submit: it closes, with both times recorded.
    git(repo.path(), &["branch", "-f", "release/desktop-0.16.4", &c]);
    let closed = r.bots[1]
        .call("release_deployed_via", via(&old, &shipped))
        .await["release"]
        .clone();
    assert_eq!(closed["status"], "deployed", "{closed}");
    let detail = closed["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "deployed_via")
        .map(|e| e["detail"].clone())
        .expect("event");
    assert_eq!(detail["via_reference"], "refs/heads/release/desktop-0.16.4");
    assert_eq!(detail["via_commit"], c.as_str(), "{detail}");
    assert!(detail["via_commit_at"].is_string(), "{detail}");
    assert!(detail["via_submitted_at"].is_string(), "{detail}");
    assert_eq!(r.column(&items[0]), "done");
}
