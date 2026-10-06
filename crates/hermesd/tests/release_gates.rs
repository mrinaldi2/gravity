//! Who may land a release on main (H-117 X2) or build its Windows installer
//! (X3), and for which commit: the daemon's gates, asked over the local
//! endpoint as the bot of the calling session. The extra × the role × the
//! release's state × the one commit its builds record (ARCH-R52 M1).

mod common;

use bus::PermissionExtra;
use common::releases::{releases, Releases};
use common::*;
use hermesd::board::release::gates::serve;
use serde_json::{json, Value};

const C1: &str = "1111111111111111111111111111111111111111";
const C2: &str = "2222222222222222222222222222222222222222";

fn ask(d: &TestDaemon, bot: &str, method: &str, params: Value) -> Result<Value, String> {
    let request = json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params});
    let reply = serve(&d.app, bot, &request).expect("a gate method");
    match reply.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or_default().to_string()),
        None => Ok(reply["result"].clone()),
    }
}

/// DevOps packages every item with one build from `commit` (or none named).
async fn package(r: &mut Releases, commit: Option<&str>) -> String {
    let items = r.items.clone();
    let created = r.bots[1]
        .call("release_create", json!({"name": "0.17.0", "items": items}))
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    let mut build = json!({"release_id": id, "platform": "daemon", "version": "0.17.0",
                           "artifact": "/builds/hermesd", "sha256": "a".repeat(64)});
    if let Some(c) = commit {
        build["source_commit"] = json!(c);
    }
    let attached = r.bots[1].call("release_attach_build", build).await;
    assert!(attached["release"].is_object(), "{attached}");
    id
}

/// Submitted and approved by the owner.
async fn approve(r: &mut Releases, id: &str) {
    r.passed(id).await;
    let release = r.bots[1]
        .call("release_submit", json!({"release_id": id}))
        .await["release"]
        .clone();
    let items: Vec<Value> = release["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| json!({"item_id": i["item_id"], "verdict": "ship"}))
        .collect();
    let mut owner = WsClient::connect(&r.pair.d).await;
    let ruled = owner
        .request(
            json!({"type": "release_rule", "release_id": id, "verdicts": items,
                        "expected_version": release["version"]}),
        )
        .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
}

#[tokio::test]
async fn landing_takes_release_main_devops_approval_and_the_built_commit() {
    let mut r = releases(1).await;
    let id = package(&mut r, Some(C1)).await;
    let d = &r.pair.d;
    let (devops, tester) = (r.pair.ids[1].clone(), r.pair.ids[2].clone());
    let land = json!({"release_id": id});

    let refused = ask(d, &devops, "hermes/release_land", land.clone()).unwrap_err();
    assert!(refused.contains("release_main extra"), "{refused}");
    for bot in [&devops, &tester] {
        d.app
            .db
            .set_bot_permission_extras(bot, &[PermissionExtra::ReleaseMain])
            .unwrap();
    }
    let refused = ask(d, &tester, "hermes/release_land", land.clone()).unwrap_err();
    assert!(refused.contains("only the project's DevOps"), "{refused}");
    let refused = ask(d, &devops, "hermes/release_land", land.clone()).unwrap_err();
    assert!(
        refused.contains("only a release the owner approved"),
        "{refused}"
    );

    approve(&mut r, &id).await;
    let d = &r.pair.d;
    let ok = ask(d, &devops, "hermes/release_land", land).unwrap();
    assert_eq!(ok["commit"], C1, "the commit the builds record: {ok}");
    // Recording anything else than that commit is refused.
    let landed =
        |commit: &str| json!({"release_id": id, "commit": commit, "tag": "desktop-v0.17.0"});
    let refused = ask(d, &devops, "hermes/release_landed", landed(C2)).unwrap_err();
    assert!(refused.contains("isn't the commit"), "{refused}");
    let recorded = ask(d, &devops, "hermes/release_landed", landed(C1)).unwrap();
    assert_eq!(recorded["recorded"], "landed");
    let got = r.bots[1]
        .call("release_get", json!({"release_id": id}))
        .await;
    let release = &got["release"];
    assert_eq!(release["builds"][0]["source_commit"], C1);
    assert!(
        release["events"].to_string().contains("desktop-v0.17.0"),
        "{got}"
    );
}

#[tokio::test]
async fn a_release_whose_builds_name_no_commit_never_lands() {
    let mut r = releases(1).await;
    let id = package(&mut r, None).await;
    approve(&mut r, &id).await;
    let d = &r.pair.d;
    let devops = r.pair.ids[1].clone();
    d.app
        .db
        .set_bot_permission_extras(&devops, &[PermissionExtra::ReleaseMain])
        .unwrap();
    let refused = ask(d, &devops, "hermes/release_land", json!({"release_id": id})).unwrap_err();
    assert!(refused.contains("record the commit"), "{refused}");
}

#[tokio::test]
async fn the_installer_is_built_from_the_packages_commit_while_it_is_open() {
    let mut r = releases(1).await;
    let id = package(&mut r, Some(C1)).await;
    let d = &r.pair.d;
    let tester = r.pair.ids[2].clone();
    let build = json!({"release_id": id});
    let refused = ask(d, &tester, "hermes/release_build_installer", build.clone()).unwrap_err();
    assert!(refused.contains("build_installers extra"), "{refused}");
    d.app
        .db
        .set_bot_permission_extras(&tester, &[PermissionExtra::BuildInstallers])
        .unwrap();
    let ok = ask(d, &tester, "hermes/release_build_installer", build.clone()).unwrap();
    assert_eq!(ok["commit"], C1, "{ok}");
    let built = |commit: &str| json!({"release_id": id, "commit": commit, "file": "x-setup.exe", "sha256": "b".repeat(64)});
    let refused = ask(d, &tester, "hermes/release_installer_built", built(C2)).unwrap_err();
    assert!(refused.contains("isn't the commit"), "{refused}");
    ask(d, &tester, "hermes/release_installer_built", built(C1)).unwrap();
    // A build from another commit can't join the package.
    let other = r.bots[1]
        .call_raw(
            "release_attach_build",
            json!({"release_id": id, "platform": "desktop-win", "version": "0.17.0",
                   "artifact": "/builds/x-setup.exe", "sha256": "b".repeat(64),
                   "source_commit": C2}),
        )
        .await;
    assert!(
        common::tasks::error_text(&other).contains("one release is built from one commit"),
        "{other}"
    );
    // Once submitted, its builds are frozen: no more installers.
    approve(&mut r, &id).await;
    let d = &r.pair.d;
    let refused = ask(d, &tester, "hermes/release_build_installer", build).unwrap_err();
    assert!(refused.contains("frozen"), "{refused}");
    // Anything else isn't a gate.
    let other = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    assert!(serve(&d.app, &tester, &other).is_none());
}
