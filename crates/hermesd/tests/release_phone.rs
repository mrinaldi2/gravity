//! Installing an iOS package on a paired phone (H-229, UX-043): the
//! desktop's Install box facts, Send to phone to one device's connections,
//! the device's waiting offer, and the build site that doesn't answer.

mod common;

use common::releases::releases_on;
use common::WsClient;
use hermesd::board::release::model::ReleaseStatus;
use serde_json::{json, Value};

/// A port nothing listens on: the build site is off.
const SITE: &str = "https://127.0.0.1:1/releases";

fn set_status(d: &common::TestDaemon, id: &str, status: ReleaseStatus) {
    d.app
        .db
        .board_tx(|t| t.set_release_status(id, status))
        .unwrap();
}

#[tokio::test]
async fn send_to_phone_reaches_only_that_device_and_waits_there() {
    let d = common::spawn_daemon().await;
    let mut r = releases_on(d, 1).await;
    let item = r.items[0].clone();
    let created = r.bots[1]
        .call(
            "release_create",
            json!({"name": "R-1", "display_version": "0.6.1", "items": [item]}),
        )
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    let install_url =
        format!("itms-services://?action=download-manifest&url={SITE}/{id}/ios/manifest.plist");
    r.bots[1]
        .call(
            "release_attach_build",
            json!({"release_id": id, "platform": "ios", "version": "12",
                   "artifact": "/builds/TheHermes.ipa", "sha256": "b".repeat(64),
                   "url": format!("{SITE}/{id}/ios/TheHermes.ipa"), "install_url": install_url}),
        )
        .await;
    let d = &r.pair.d;
    let mut owner = WsClient::connect(d).await;
    let info =
        |check: bool| json!({"type": "release_install", "release_id": id, "check_site": check});

    // Being packaged: no install actions yet.
    let early = owner.request(info(false)).await;
    assert_eq!(early["type"], "release_install", "{early}");
    assert_eq!(early["install"]["installable"], Value::Null, "{early}");

    // Awaiting the owner: offered for testing; the site is off, and no
    // device is paired yet.
    set_status(d, &id, ReleaseStatus::AwaitingOwner);
    let got = owner.request(info(true)).await;
    let install = &got["install"];
    assert_eq!(install["installable"], true, "{got}");
    assert_eq!(install["for_testing"], true, "{got}");
    assert_eq!(install["version"], "0.6.1");
    assert_eq!(install["build"], "12");
    assert_eq!(install["page_url"], format!("{SITE}/{id}/ios/index.html"));
    assert_eq!(install["install_url"], install_url.as_str());
    assert_eq!(
        install["site"]["serving"],
        Value::Null,
        "not serving: {got}"
    );
    assert!(
        install["site"]["problem"]
            .as_str()
            .is_some_and(|p| !p.is_empty()),
        "{got}"
    );
    assert!(
        install["computer"].as_str().is_some_and(|c| !c.is_empty()),
        "{got}"
    );
    assert_eq!(install["devices"], Value::Null, "none paired: {got}");
    let send = |device: &str| json!({"type": "release_send_to_device", "release_id": id, "device_id": device});
    let none = owner.request(send("nope")).await;
    assert_eq!(none["code"], "not_found", "{none}");

    // A paired, connected phone is listed and gets the link.
    let paired = owner
        .request(json!({"type": "create_device", "name": "iPhone 16",
                        "capabilities": ["read", "control"]}))
        .await;
    let device_id = paired["device"]["id"].as_str().unwrap().to_string();
    let mut phone = WsClient::connect_as(d, paired["token"].as_str().unwrap()).await;
    let listed = owner.request(info(false)).await;
    let device = &listed["install"]["devices"][0];
    assert_eq!(device["name"], "iPhone 16", "{listed}");
    assert_eq!(device["connected"], true, "{listed}");

    let sent = owner.request(send(&device_id)).await;
    assert_eq!(sent["type"], "install_offer", "{sent}");
    let offer = &sent["offer"];
    assert_eq!(offer["title"], "The Hermes 0.6.1 (12) is ready to install");
    assert_eq!(offer["body"], "Ready to test. Tap to install.");
    assert_eq!(offer["install_url"], install_url.as_str());
    assert_eq!(offer["delivered"], true);
    assert_eq!(offer["device_name"], "iPhone 16");

    let pushed = phone
        .wait_for(|v| v["type"] == "install_offer" && v["req_id"].is_null())
        .await;
    assert_eq!(pushed["device_id"], device_id.as_str(), "{pushed}");
    assert_eq!(pushed["offer"]["release_id"], id.as_str(), "{pushed}");

    // The offer waits on the phone until Not now; the desktop has none.
    let waiting = phone.request(json!({"type": "install_offers"})).await;
    assert_eq!(waiting["offers"][0]["release_id"], id.as_str(), "{waiting}");
    let mine = owner.request(json!({"type": "install_offers"})).await;
    assert_eq!(mine["offers"], json!([]), "{mine}");
    let refused = owner
        .request(json!({"type": "install_offer_dismiss", "release_id": id}))
        .await;
    assert_eq!(refused["code"], "forbidden", "{refused}");
    let dismissed = phone
        .request(json!({"type": "install_offer_dismiss", "release_id": id}))
        .await;
    assert_eq!(dismissed["offers"], json!([]), "{dismissed}");

    // A phone that isn't connected still gets it, the next time it opens.
    drop(phone);
    let offline = wait_offline(&mut owner, &id).await;
    let later = owner.request(send(&device_id)).await;
    assert_eq!(
        later["offer"]["delivered"],
        Value::Null,
        "{later} after {offline}"
    );
    let mut phone = WsClient::connect_as(d, paired["token"].as_str().unwrap()).await;
    let waiting = phone.request(json!({"type": "install_offers"})).await;
    assert_eq!(waiting["offers"][0]["release_id"], id.as_str(), "{waiting}");

    // Rejected: nothing to install, and nothing is sent.
    set_status(d, &id, ReleaseStatus::Rejected);
    let refused = owner.request(send(&device_id)).await;
    assert_eq!(refused["code"], "conflict", "{refused}");
}

/// Waits until the daemon has seen the phone's connection close.
async fn wait_offline(owner: &mut WsClient, id: &str) -> Value {
    for _ in 0..50 {
        let got = owner
            .request(json!({"type": "release_install", "release_id": id}))
            .await;
        if got["install"]["devices"][0]["connected"].is_null() {
            return got;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the phone still shows as connected");
}
