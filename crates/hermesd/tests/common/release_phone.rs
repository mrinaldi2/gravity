//! A release with an iPhone build published into the served folder, as
//! the H-229 install tests start from.

use std::path::PathBuf;

use bus::PermissionExtra;
use hermesd::board::release::model::ReleaseStatus;
use serde_json::json;

use super::releases::{releases_on, Releases};

/// A port nothing listens on: the build site is off.
pub const SITE: &str = "https://127.0.0.1:1/releases";

pub fn set_status(d: &super::TestDaemon, id: &str, status: ReleaseStatus) {
    d.app
        .db
        .board_tx(|t| t.set_release_status(id, status))
        .unwrap();
}

/// A release with an iPhone build DevOps published into the served folder:
/// its id, the served folder and the build's sha256.
pub async fn published() -> (Releases, String, PathBuf, String) {
    let d = super::spawn_daemon_with(|cfg| {
        cfg.releases.dir = Some(cfg.home.join("releases"));
        cfg.releases.base_url = Some(SITE.into());
        cfg.releases.source_roots = vec![cfg.home.join("builds").display().to_string()];
    })
    .await;
    let home = d.app.cfg.home.clone();
    let mut r = releases_on(d, 1).await;
    let builds = home.join("builds");
    std::fs::create_dir_all(&builds).unwrap();
    let ipa = builds.join("TheHermes.ipa");
    std::fs::write(&ipa, "ipa bytes").unwrap();
    r.pair
        .d
        .app
        .db
        .set_bot_permission_extras(&r.pair.ids[1], &[PermissionExtra::Publish])
        .unwrap();
    let item = r.items[0].clone();
    let created = r.bots[1]
        .call(
            "release_create",
            json!({"name": "R-1", "display_version": "0.6.1", "items": [item]}),
        )
        .await;
    let id = created["release"]["id"].as_str().unwrap().to_string();
    let done = r.bots[1]
        .call(
            "release_publish",
            json!({"release_id": id, "file": ipa.display().to_string(),
                   "version": "12", "bundle_id": "com.example.hermes"}),
        )
        .await;
    let sha = done["published"]["sha256"].as_str().unwrap().to_string();
    let served = std::fs::canonicalize(home.join("releases").join(&id).join("ios")).unwrap();
    (r, id, served, sha)
}

/// The page and install links this daemon derives for the release.
pub fn ours(id: &str) -> (String, String) {
    (
        format!("{SITE}/{id}/ios/index.html"),
        format!("itms-services://?action=download-manifest&url={SITE}/{id}/ios/manifest.plist"),
    )
}
