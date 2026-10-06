//! Who may land a release on main (H-117 X2) or build its Windows installer
//! (X3): the daemon's gates, asked over the local endpoint as the bot of the
//! calling session. The extra × the role × an owner-approved release.

mod common;

use bus::PermissionExtra;
use common::releases::releases;
use common::*;
use hermesd::board::release::gates::serve;
use serde_json::{json, Value};

fn ask(d: &TestDaemon, bot: &str, method: &str, params: Value) -> Result<Value, String> {
    let request = json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params});
    let reply = serve(&d.app, bot, &request).expect("a gate method");
    match reply.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or_default().to_string()),
        None => Ok(reply["result"].clone()),
    }
}

#[tokio::test]
async fn landing_takes_release_main_devops_and_an_approved_release() {
    let mut r = releases(1).await;
    let release = r.submitted("0.17.0").await;
    let id = release["id"].as_str().unwrap().to_string();
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
    // Waiting for the owner: not yet.
    let refused = ask(d, &devops, "hermes/release_land", land.clone()).unwrap_err();
    assert!(
        refused.contains("only a release the owner approved"),
        "{refused}"
    );

    let items: Vec<Value> = release["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| json!({"item_id": i["item_id"], "verdict": "ship"}))
        .collect();
    let mut owner = WsClient::connect(d).await;
    let ruled = owner
        .request(
            json!({"type": "release_rule", "release_id": id, "verdicts": items,
                        "expected_version": release["version"]}),
        )
        .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");

    let ok = ask(d, &devops, "hermes/release_land", land).unwrap();
    assert_eq!(ok["release_id"], json!(id));
    assert!(ok["version"].is_string(), "{ok}");
    let recorded = ask(
        d,
        &devops,
        "hermes/release_landed",
        json!({"release_id": id, "commit": "abc123", "tag": "desktop-v0.17.0"}),
    )
    .unwrap();
    assert_eq!(recorded["recorded"], "landed");
    let got = r.bots[1]
        .call("release_get", json!({"release_id": id}))
        .await;
    let events = got["release"]["events"].to_string();
    assert!(
        events.contains("landed") && events.contains("desktop-v0.17.0"),
        "{got}"
    );

    // Building the installer is its own extra.
    let build = json!({"release_id": id});
    let refused = ask(d, &tester, "hermes/release_build_installer", build.clone()).unwrap_err();
    assert!(refused.contains("build_installers extra"), "{refused}");
    d.app
        .db
        .set_bot_permission_extras(&tester, &[PermissionExtra::BuildInstallers])
        .unwrap();
    ask(d, &tester, "hermes/release_build_installer", build).unwrap();
    // Anything else isn't a gate.
    let other = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    assert!(serve(&d.app, &tester, &other).is_none());
}
