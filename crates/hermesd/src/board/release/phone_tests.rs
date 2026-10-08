//! The install page, its url, and what a device's notification says (H-229).

use bus::contract::home::{InstallDevice, ReleaseInstall};

use super::{installable, offer, page_html, page_url};
use crate::board::release::model::ReleaseStatus;

#[test]
fn the_page_sits_beside_the_build() {
    assert_eq!(
        page_url("https://mac.tail.ts.net/releases/r1/ios/TheHermes.ipa").as_deref(),
        Some("https://mac.tail.ts.net/releases/r1/ios/index.html")
    );
    assert_eq!(
        page_url("https://mac.tail.ts.net/ios/0.6.1-12/index.html").as_deref(),
        Some("https://mac.tail.ts.net/ios/0.6.1-12/index.html"),
        "a page url is kept as it is"
    );
    assert_eq!(page_url("http://mac.tail.ts.net/r1/ios/a.ipa"), None);
    assert_eq!(page_url("/builds/TheHermes.ipa"), None);
    assert_eq!(page_url("https://"), None);
}

#[test]
fn the_page_links_the_install_and_escapes_what_it_shows() {
    let link = "itms-services://?action=download-manifest&url=https://m/r/ios/manifest.plist";
    let html = page_html("The <Hermes>", "0.6.1", link);
    assert!(
        html.contains(r#"href="itms-services://?action=download-manifest&amp;url=https://m/r/ios/manifest.plist""#),
        "{html}"
    );
    assert!(html.contains("The &lt;Hermes&gt; 0.6.1"), "{html}");
    assert!(
        html.contains("this device may not be in the build's profile"),
        "{html}"
    );
}

#[test]
fn install_actions_show_while_tested_approved_rolling_out_or_live() {
    use ReleaseStatus::*;
    for s in [AwaitingOwner, Approved, Deploying, Deployed] {
        assert!(installable(s), "{s:?}");
    }
    for s in [
        Planned, Assembling, Built, Held, Rejected, Superseded, RolledBack,
    ] {
        assert!(!installable(s), "{s:?}");
    }
}

fn info(for_testing: bool, approved: Option<i64>) -> ReleaseInstall {
    ReleaseInstall {
        release_id: "r1".into(),
        project_id: "p".into(),
        version: "0.6.1".into(),
        build: "12".into(),
        state: if for_testing {
            "awaiting_owner"
        } else {
            "approved"
        }
        .into(),
        installable: true,
        for_testing,
        page_url: "https://m/r1/ios/index.html".into(),
        install_url: "itms-services://x".into(),
        app_title: "The Hermes".into(),
        approved_at: approved
            .map(|seconds| bus::contract::pbjson_types::Timestamp { seconds, nanos: 0 }),
        ..Default::default()
    }
}

fn phone(connected: bool) -> InstallDevice {
    InstallDevice {
        device_id: "d1".into(),
        name: "iPhone 16".into(),
        connected,
        ..Default::default()
    }
}

#[test]
fn the_notification_names_the_version_and_why_it_waits() {
    let testing = offer(&info(true, None), &phone(true));
    assert_eq!(testing.title, "The Hermes 0.6.1 (12) is ready to install");
    assert_eq!(testing.body, "Ready to test. Tap to install.");
    assert!(testing.delivered && testing.for_testing);
    assert_eq!(testing.install_url, "itms-services://x");
    assert_eq!(testing.device_name, "iPhone 16");

    // 2026-10-08T12:00:00Z
    let approved = offer(&info(false, Some(1_791_460_800)), &phone(false));
    assert_eq!(approved.body, "Approved on 8 Oct 2026. Tap to install.");
    assert!(!approved.delivered);

    let mut same = info(false, None);
    same.build = "0.6.1".into();
    let plain = offer(&same, &phone(true));
    assert_eq!(plain.title, "The Hermes 0.6.1 is ready to install");
    assert_eq!(plain.body, "Tap to install.");
}
