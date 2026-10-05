//! Serving builds on the tailnet end to end (B7c, H-020 §6.6): DevOps with
//! the Publish extra stages builds into a temporary served directory, and
//! `install_release` refuses a served build that no longer matches its
//! frozen sha256.

mod common;

use bus::PermissionExtra;
use common::releases::{releases_on, rule};
use common::tasks::error_text;
use common::WsClient;
use serde_json::json;

const BASE: &str = "https://mac.tail.ts.net/releases";

#[tokio::test]
async fn devops_publishes_and_installs_check_the_served_file() {
    let d = common::spawn_daemon_with(|cfg| {
        cfg.releases.dir = Some(cfg.home.join("served"));
        cfg.releases.base_url = Some(BASE.into());
        cfg.releases.source_roots = vec![cfg.home.join("builds").display().to_string()];
    })
    .await;
    let home = d.app.cfg.home.clone();
    let mut r = releases_on(d, 1).await;
    let builds = home.join("builds");
    std::fs::create_dir_all(&builds).unwrap();
    let ipa = builds.join("TheHermes.ipa");
    std::fs::write(&ipa, "ipa bytes").unwrap();
    let outside = home.join("secret.ipa");
    std::fs::write(&outside, "not a build").unwrap();

    let item = r.items[0].clone();
    let created = r.bots[1]
        .call("release_create", json!({"name": "0.16.0", "items": [item]}))
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    let publish = json!({"release_id": id, "file": ipa.display().to_string(),
                         "version": "0.16.0", "bundle_id": "com.example.hermes"});

    // Only DevOps, and only with the Publish extra.
    let raw = r.bots[0].call_raw("release_publish", publish.clone()).await;
    assert!(error_text(&raw).contains("role"), "{raw}");
    let raw = r.bots[1].call_raw("release_publish", publish.clone()).await;
    assert!(
        error_text(&raw).contains("Publish permission extra"),
        "{raw}"
    );
    let db = &r.pair.d.app.db;
    db.set_bot_permission_extras(&r.pair.ids[1], &[PermissionExtra::Publish])
        .unwrap();

    let ops = &mut r.bots[1];
    let raw = ops
        .call_raw(
            "release_publish",
            json!({"release_id": id, "file": outside.display().to_string(), "version": "1",
                   "bundle_id": "b"}),
        )
        .await;
    assert!(error_text(&raw).contains("outside the roots"), "{raw}");

    let done = ops.call("release_publish", publish.clone()).await;
    let p = &done["published"];
    let sha = p["sha256"].as_str().unwrap().to_string();
    assert_eq!(p["url"], format!("{BASE}/{id}/ios/TheHermes.ipa"));
    assert_eq!(
        p["install_url"],
        format!("itms-services://?action=download-manifest&url={BASE}/{id}/ios/manifest.plist")
    );
    let build = &done["release"]["builds"][0];
    assert_eq!(build["platform"], "ios");
    assert_eq!(build["sha256"], sha.as_str());
    assert_eq!(build["url"], p["url"]);
    let served = home.join("served").join(&id).join("ios");
    assert_eq!(
        std::fs::read_to_string(served.join("TheHermes.ipa")).unwrap(),
        "ipa bytes"
    );
    let manifest = std::fs::read_to_string(served.join("manifest.plist")).unwrap();
    assert!(manifest.contains("com.example.hermes"), "{manifest}");

    // Publishing again is a no-op; a different file under the same name is refused.
    let again = ops.call("release_publish", publish.clone()).await;
    assert_eq!(again["published"]["already_published"], true);
    std::fs::write(&ipa, "rebuilt").unwrap();
    let raw = ops.call_raw("release_publish", publish.clone()).await;
    assert!(error_text(&raw).contains("different content"), "{raw}");

    // The item is a daemon one, so the mac tester reports on the daemon
    // build, never on the iOS one (B7b-m1 F2).
    let daemon = builds.join("hermesd.zip");
    std::fs::write(&daemon, "daemon bytes").unwrap();
    let mac = ops
        .call(
            "release_publish",
            json!({"release_id": id, "file": daemon.display().to_string(),
                   "platform": "daemon", "version": "0.16.0"}),
        )
        .await["published"]["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    let test = |sha: &str| json!({"release_id": id, "machine": "mac", "build_sha256": sha, "result": "pass"});
    let raw = r.bots[2].call_raw("release_test", test(&sha)).await;
    assert!(error_text(&raw).contains("is the ios build"), "{raw}");

    // Test, submit, rule, deploy: the tester's install checks the served files.
    r.bots[2].call("release_test", test(&mac)).await;
    let release = r.bots[1]
        .call("release_submit", json!({"release_id": id}))
        .await["release"]
        .clone();
    let raw = r.bots[1].call_raw("release_publish", publish).await;
    assert!(error_text(&raw).contains("before it is submitted"), "{raw}");
    let mut owner = WsClient::connect(&r.pair.d).await;
    let ruled = rule(
        &mut owner,
        &release,
        json!([{"item_id": item, "verdict": "ship"}]),
    )
    .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    r.bots[1]
        .call(
            "release_deploy",
            json!({"release_id": id, "machine": "mac"}),
        )
        .await;
    let install = json!({"release_id": id});
    let got = r.bots[2].call("install_release", install.clone()).await;
    assert_eq!(got["builds"][0]["verified"], true, "{got}");
    assert_eq!(got["builds"][1]["verified"], true, "{got}");

    // A changed served file refuses the install and pauses the rollout.
    std::fs::write(served.join("TheHermes.ipa"), "tampered").unwrap();
    let raw = r.bots[2].call_raw("install_release", install).await;
    assert!(error_text(&raw).contains("no longer matches"), "{raw}");
    let now = r.bots[1]
        .call("release_get", json!({"release_id": id}))
        .await;
    assert_eq!(now["release"]["status"], "paused", "{now}");
    let reason = now["release"]["paused_reason"].as_str().unwrap();
    assert!(reason.contains("sha256"), "{reason}");
}
