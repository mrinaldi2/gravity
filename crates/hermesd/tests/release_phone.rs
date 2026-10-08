//! Installing an iOS package on a paired phone (H-229, UX-043): the
//! desktop's Install box facts, Send to phone to one device's connections,
//! the device's waiting offer, and the build site that doesn't answer. The
//! links are the daemon's own, derived from the file it serves, never the
//! ones a bot wrote on the build row (CE review of H-229, M1); only the app
//! or a paired device sends one (M2).

mod common;

use std::io::ErrorKind;
use std::net::TcpListener;

use common::release_phone::{ours, published, set_status};
use common::releases::Releases;
use common::WsClient;
use hermesd::board::release::model::ReleaseStatus;
use serde_json::{json, Value};

#[tokio::test]
async fn send_to_phone_reaches_only_that_device_and_waits_there() {
    let (r, id, _, _) = published().await;
    let (page_url, install_url) = ours(&id);
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
    assert_eq!(install["page_url"], page_url.as_str(), "{got}");
    assert_eq!(install["install_url"], install_url.as_str(), "{got}");
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

    // The owner token a bot can read doesn't send it (M2).
    let mut forger = WsClient::connect_owner_token(d).await;
    let forged = forger.request(send(&device_id)).await;
    assert_eq!(forged["code"], "forbidden", "{forged}");
    let waiting = phone.request(json!({"type": "install_offers"})).await;
    assert_eq!(waiting["offers"], json!([]), "nothing sent: {waiting}");

    let sent = owner.request(send(&device_id)).await;
    assert_eq!(sent["type"], "install_offer", "{sent}");
    let offer = &sent["offer"];
    assert_eq!(offer["title"], "The Hermes 0.6.1 (12) is ready to install");
    assert_eq!(offer["body"], "Ready to test. Tap to install.");
    assert_eq!(offer["install_url"], install_url.as_str());
    assert_eq!(offer["page_url"], page_url.as_str());
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

/// A build row is written by a bot: its `url` and `install_url` are never
/// shown, sent, written into our page or probed. A row whose file isn't the
/// one served as built gets no link at all.
#[tokio::test]
async fn a_build_row_never_chooses_the_link() {
    let (mut r, id, served, sha) = published().await;
    let (page_url, install_url) = ours(&id);
    // The foreign host: anything that connects here was probed.
    let foreign = TcpListener::bind("127.0.0.1:0").unwrap();
    foreign.set_nonblocking(true).unwrap();
    let evil = format!("https://{}", foreign.local_addr().unwrap());
    let attach = |artifact: &str, sha: &str| {
        json!({"release_id": id, "platform": "ios", "version": "12",
               "artifact": artifact, "sha256": sha,
               "url": format!("{evil}/x/index.html"),
               "install_url": format!("itms-services://?action=download-manifest&url={evil}/m.plist")})
    };
    let ipa = served.join("TheHermes.ipa").display().to_string();
    let page = served.join("index.html");
    let mut owner = WsClient::connect(&r.pair.d).await;
    let paired = owner
        .request(json!({"type": "create_device", "name": "iPhone 16",
                        "capabilities": ["read", "control"]}))
        .await;
    let device_id = paired["device"]["id"].as_str().unwrap().to_string();
    let info = json!({"type": "release_install", "release_id": id, "check_site": true});
    let send = json!({"type": "release_send_to_device", "release_id": id, "device_id": device_id});
    let not_probed = |foreign: &TcpListener| {
        let accepted = foreign.accept();
        assert!(
            accepted
                .as_ref()
                .is_err_and(|e| e.kind() == ErrorKind::WouldBlock),
            "the foreign host was probed: {accepted:?}"
        );
    };

    // The served file, as built, under foreign links: ours are used at all
    // three points (the answer and the probe, the page, the offer).
    std::fs::remove_file(&page).unwrap();
    r.bots[1]
        .call("release_attach_build", attach(&ipa, &sha))
        .await;
    set_status(&r.pair.d, &id, ReleaseStatus::AwaitingOwner);
    let got = owner.request(info.clone()).await;
    assert_eq!(got["install"]["page_url"], page_url.as_str(), "{got}");
    assert_eq!(got["install"]["install_url"], install_url.as_str(), "{got}");
    assert!(!got.to_string().contains(&evil), "{got}");
    not_probed(&foreign);
    let written = std::fs::read_to_string(&page).unwrap();
    assert!(!written.contains(&evil), "{written}");
    assert!(
        written.contains(&install_url.replace('&', "&amp;")),
        "{written}"
    );
    let sent = owner.request(send.clone()).await;
    assert_eq!(sent["offer"]["install_url"], install_url.as_str(), "{sent}");
    assert_eq!(sent["offer"]["page_url"], page_url.as_str(), "{sent}");

    // A file this daemon doesn't serve, and the served file under another
    // sha: no install info, nothing sent, no page written, nothing probed.
    let outside = r.pair.d.app.cfg.home.join("builds/TheHermes.ipa");
    for (artifact, sha) in [
        (outside.display().to_string(), sha.clone()),
        (ipa.clone(), "b".repeat(64)),
    ] {
        std::fs::remove_file(&page).ok();
        set_status(&r.pair.d, &id, ReleaseStatus::Built);
        r.bots[1]
            .call("release_attach_build", attach(&artifact, &sha))
            .await;
        set_status(&r.pair.d, &id, ReleaseStatus::AwaitingOwner);
        let got = owner.request(info.clone()).await;
        let install = &got["install"];
        assert_eq!(install["installable"], true, "{artifact}: {got}");
        assert_eq!(install["page_url"], Value::Null, "{artifact}: {got}");
        assert_eq!(install["install_url"], Value::Null, "{artifact}: {got}");
        assert_eq!(install["site"], Value::Null, "not probed: {got}");
        not_probed(&foreign);
        assert!(!page.exists(), "{artifact}: a page was written");
        let refused = owner.request(send.clone()).await;
        assert_eq!(refused["code"], "conflict", "{artifact}: {refused}");
        let waiting = waiting_offer(&r, &device_id);
        assert!(!waiting.contains(&evil), "{waiting}");
    }
}

/// A paired phone's hello says what it runs (H-241): the desktop shows it,
/// bounded and printable, and it never creates or confirms a deploy record,
/// even when it names the very build being rolled out.
#[tokio::test]
async fn a_phone_reports_its_version_for_display_only() {
    let (r, id, _, _) = published().await;
    let d = &r.pair.d;
    set_status(d, &id, ReleaseStatus::Deploying);
    let release = || d.app.db.board_read(|t| t.release(&id)).unwrap().unwrap();
    let before = release();
    let mut owner = WsClient::connect(d).await;
    let paired = owner
        .request(json!({"type": "create_device", "name": "iPhone 16",
                        "capabilities": ["read", "control"]}))
        .await;
    let token = paired["token"].as_str().unwrap();
    assert_eq!(
        reported(&mut owner, &id).await,
        (Value::Null, Value::Null),
        "not yet"
    );

    // The owner's own connection reports nothing for a device.
    WsClient::connect_hello(d, &d.app.owner.mint(), json!({"app_version": "9.9.9"})).await;
    assert_eq!(reported(&mut owner, &id).await.0, Value::Null);

    let hello = |version: Value, build: Value| json!({"app_version": version, "app_build": build});
    WsClient::connect_hello(d, token, hello(json!(" 0.6.1 "), json!("12"))).await;
    let (version, seen) = reported(&mut owner, &id).await;
    assert_eq!(version, "0.6.1 (12)");
    assert!(seen.as_str().is_some_and(|s| !s.is_empty()), "{seen}");

    // A hello without a usable version keeps the last report.
    WsClient::connect_as(d, token).await;
    let too_long = json!("9".repeat(33));
    WsClient::connect_hello(d, token, hello(too_long, json!("13"))).await;
    WsClient::connect_hello(d, token, hello(json!("0.7\u{202e}"), json!("13"))).await;
    WsClient::connect_hello(d, token, hello(json!(7), json!("13"))).await;
    assert_eq!(reported(&mut owner, &id).await.0, "0.6.1 (12)");
    // A bad build alone is left out.
    WsClient::connect_hello(d, token, hello(json!("0.6.2"), json!("1\n3"))).await;
    assert_eq!(reported(&mut owner, &id).await.0, "0.6.2");

    // Display only: the package, its deploy records and its events are as
    // they were.
    assert_eq!(release(), before);
    assert!(before.deployments.is_empty(), "{:?}", before.deployments);
}

/// The first paired device's reported version and when it said so.
async fn reported(owner: &mut WsClient, id: &str) -> (Value, Value) {
    let got = owner
        .request(json!({"type": "release_install", "release_id": id}))
        .await;
    let device = &got["install"]["devices"][0];
    assert_eq!(device["name"], "iPhone 16", "{got}");
    (
        device["app_version"].clone(),
        device["app_version_seen_at"].clone(),
    )
}

/// The device's waiting offer as stored.
fn waiting_offer(r: &Releases, device_id: &str) -> String {
    r.pair
        .d
        .app
        .db
        .get_meta(&format!("install_offer:{device_id}"))
        .unwrap()
        .unwrap_or_default()
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
