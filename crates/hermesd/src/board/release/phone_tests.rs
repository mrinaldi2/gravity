//! The install page, its url, and what a device's notification says (H-229).

use std::fs;
use std::path::{Path, PathBuf};

use bus::contract::home::{InstallDevice, ReleaseInstall};

use super::{installable, links, offer, page_html, serve};
use crate::board::release::model::{ReleaseBuild, ReleaseStatus};
use crate::config::Config;

const BASE: &str = "https://mac.tail.ts.net/releases";

/// A config serving `<tmp>/served` at BASE, with `r1/ios/TheHermes.ipa`
/// and its manifest published there; returns the build's path and sha.
fn served(tmp: &Path) -> (Config, PathBuf, String) {
    let mut cfg = Config {
        home: tmp.to_path_buf(),
        ..Config::default()
    };
    cfg.releases.dir = Some(tmp.join("served"));
    cfg.releases.base_url = Some(format!("{BASE}/"));
    let dir = tmp.join("served/r1/ios");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("TheHermes.ipa"), "ipa bytes").unwrap();
    fs::write(dir.join("manifest.plist"), "plist").unwrap();
    let path = fs::canonicalize(dir.join("TheHermes.ipa")).unwrap();
    let sha = serve::sha256_file(&path).unwrap();
    (cfg, path, sha)
}

/// A build row as a bot might write it: foreign links on any artifact.
fn row(artifact: &Path, sha256: &str) -> ReleaseBuild {
    ReleaseBuild {
        platform: "ios".into(),
        version: "12".into(),
        artifact: artifact.display().to_string(),
        url: Some("https://evil.example/r1/ios/TheHermes.ipa".into()),
        install_url: Some(
            "itms-services://?action=download-manifest&url=https://evil.example/m.plist".into(),
        ),
        sha256: sha256.into(),
        built_at: chrono::Utc::now(),
        source_commit: None,
    }
}

#[test]
fn links_come_from_the_served_file_never_the_row() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, path, sha) = served(tmp.path());
    let got = links(&cfg, "r1", &row(&path, &sha)).expect("a served build");
    assert_eq!(got.page, format!("{BASE}/r1/ios/index.html"));
    assert_eq!(
        got.install,
        format!("itms-services://?action=download-manifest&url={BASE}/r1/ios/manifest.plist")
    );
    assert_eq!(got.dir, path.parent().unwrap());
}

#[test]
fn a_build_not_served_as_built_has_no_links() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, path, sha) = served(tmp.path());
    // The sha on the row isn't the served file's.
    assert_eq!(links(&cfg, "r1", &row(&path, &"b".repeat(64))), None);
    // A file outside the served root.
    let outside = tmp.path().join("TheHermes.ipa");
    fs::write(&outside, "ipa bytes").unwrap();
    assert_eq!(links(&cfg, "r1", &row(&outside, &sha)), None);
    // Another release's folder.
    assert_eq!(links(&cfg, "r2", &row(&path, &sha)), None);
    // A path that climbs out and back in, or goes through a symlink.
    let climbing = path.parent().unwrap().join("../ios/TheHermes.ipa");
    assert_eq!(links(&cfg, "r1", &row(&climbing, &sha)), None);
    #[cfg(unix)]
    {
        let link = path.parent().unwrap().join("Linked.ipa");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(links(&cfg, "r1", &row(&link, &sha)), None);
    }
    // No base_url configured.
    let mut unset = cfg.clone();
    unset.releases.base_url = None;
    assert_eq!(links(&unset, "r1", &row(&path, &sha)), None);
    // No manifest beside it.
    fs::remove_file(path.parent().unwrap().join("manifest.plist")).unwrap();
    assert_eq!(links(&cfg, "r1", &row(&path, &sha)), None);
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
