//! Installing an iOS package on a paired iPhone or iPad (H-229, UX-043):
//! the HTTPS install page beside the build, whether the build site answers,
//! and the offer a device gets when the owner sends it the link.
//!
//! An offer is the device's one waiting install link, kept in `meta` under
//! `install_offer:<device id>` until the device dismisses it or a newer one
//! replaces it, so a phone that wasn't connected sees it when it next opens.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bus::contract::home::{InstallDevice, InstallOffer, ReleaseInstall, SiteCheck};
use bus::DecisionState;

use super::model::{Release, ReleaseBuild, ReleaseStatus};
use super::{confine, machines, phone_version, serve, sha_cache};
use crate::app::AppState;
use crate::attention::timestamp;
use crate::config::Config;
use crate::decisions::{conflict, invalid, not_found};

/// The install page's file name, next to the build it installs.
pub const PAGE: &str = "index.html";

/// How long the build site has to answer before it counts as off.
const SITE_WAIT: Duration = Duration::from_secs(4);

/// The states that offer install actions (UX-043 §1): being tested by the
/// owner, approved, rolling out, or live.
pub fn installable(status: ReleaseStatus) -> bool {
    matches!(
        status,
        ReleaseStatus::AwaitingOwner
            | ReleaseStatus::Approved
            | ReleaseStatus::Deploying
            | ReleaseStatus::Deployed
    )
}

/// The package's iOS build, if it has one.
pub fn ios_build(release: &Release) -> Option<&ReleaseBuild> {
    release.builds.iter().find(|b| b.platform == "ios")
}

/// The install page: one Install button for the itms-services link.
pub fn page_html(title: &str, version: &str, install_url: &str) -> String {
    let [title, version, link] = [title, version, install_url].map(serve::xml_escape);
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Install {title} {version}</title>
<style>body{{font:17px -apple-system,system-ui,sans-serif;margin:48px 20px;text-align:center}}
a{{display:inline-block;padding:14px 32px;border-radius:12px;background:#0a84ff;color:#fff;font-weight:600;text-decoration:none}}
p{{color:#555}}</style></head>
<body><h1>{title} {version}</h1>
<p><a href="{link}">Install</a></p>
<p>Open this page in Safari on the iPhone or iPad, on your tailnet.</p>
<p>If iOS says it can't install, this device may not be in the build's profile. Ask DevOps.</p>
</body></html>
"#
    )
}

/// A build's install links, derived by this daemon (CE review of H-229,
/// M1): never the `url` or `install_url` on the build row, which a bot
/// writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Links {
    /// The HTTPS install page, under the configured `base_url`.
    pub page: String,
    /// The `itms-services` link to the manifest beside the build.
    pub install: String,
    /// The served folder the build, its manifest and its page sit in.
    pub dir: PathBuf,
}

/// The links for a build this daemon serves, or `None`. The build's file
/// must be a regular file at `<served root>/<release>/ios/<name>`, with no
/// symlink on the way, beside its `manifest.plist`, and still hash to the
/// build's sha256; the URLs are then `base_url` plus that folder. Anything
/// else gets no install link: nothing is shown, sent, written or probed.
pub fn links(cfg: &Config, release_id: &str, build: &ReleaseBuild) -> Option<Links> {
    let path = serve::served_file(cfg, &build.artifact)?;
    let root = std::fs::canonicalize(serve::served_root(cfg)).ok()?;
    if std::fs::canonicalize(&path).ok()? != path {
        return None;
    }
    let parts: Vec<&str> = path
        .strip_prefix(&root)
        .ok()?
        .iter()
        .map(|p| p.to_str())
        .collect::<Option<_>>()?;
    let [release, "ios", name] = parts[..] else {
        return None;
    };
    if release != release_id || !serve::is_safe_name(release) || !serve::is_safe_name(name) {
        return None;
    }
    let dir = path.parent()?.to_path_buf();
    let manifest = std::fs::symlink_metadata(dir.join("manifest.plist")).ok()?;
    if !manifest.is_file() || !sha_cache::SHAS.matches(&path, &build.sha256) {
        return None;
    }
    let folder = format!("{}/{release}/ios", serve::base_url(cfg).ok()?);
    Some(Links {
        page: format!("{folder}/{PAGE}"),
        install: format!("itms-services://?action=download-manifest&url={folder}/manifest.plist"),
        dir,
    })
}

/// Writes the install page beside a build this daemon serves, if it isn't
/// there yet: builds published before H-229 have none.
pub fn ensure_page(app: &AppState, version: &str, links: &Links) -> anyhow::Result<()> {
    if links.dir.join(PAGE).exists() {
        return Ok(());
    }
    let dirs = confine::prepare(&app.cfg)?;
    let title = app
        .cfg
        .releases
        .ios_title
        .as_deref()
        .unwrap_or("The Hermes");
    serve::write_page(
        &dirs,
        &links.dir,
        &page_html(title, version, &links.install),
    )
}

/// Asks the build site for the install page over HTTPS, with curl as the
/// installer fetches builds (no TLS stack in the daemon).
pub async fn check_site(page_url: &str) -> SiteCheck {
    let wait = SITE_WAIT.as_secs().to_string();
    let answer = tokio::process::Command::new("curl")
        .args([
            "-sS",
            "-o",
            NULL_DEVICE,
            "-w",
            "%{http_code}",
            "--proto",
            "=https",
        ])
        .args(["--max-time", wait.as_str(), "--", page_url])
        .kill_on_drop(true)
        .output()
        .await;
    let (serving, problem) = match answer {
        Ok(out) => {
            let code = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if code.starts_with('2') {
                (true, String::new())
            } else if code.is_empty() || code == "000" {
                let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
                (false, format!("no answer: {why}"))
            } else {
                (false, format!("the install page answered {code}"))
            }
        }
        Err(e) => (false, format!("couldn't run curl: {e}")),
    };
    SiteCheck {
        serving,
        problem,
        checked_at: Some(timestamp(bus::now())),
    }
}

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

/// What the package offers for install, as of now (the site unchecked).
pub fn info(app: &AppState, release: &Release) -> anyhow::Result<ReleaseInstall> {
    let build = ios_build(release);
    let links = build.and_then(|b| links(&app.cfg, &release.id, b));
    if let (Some(b), Some(l)) = (build, &links) {
        // A missing page only costs the Copy link its target; say so in the
        // log rather than fail the whole answer.
        if let Err(e) = ensure_page(app, &b.version, l) {
            tracing::warn!(release = %release.id, "couldn't write the install page: {e:#}");
        }
    }
    let (page, install_url) = links.map(|l| (l.page, l.install)).unwrap_or_default();
    let computer = app.db.board_read(machines::this_computer)?;
    Ok(ReleaseInstall {
        release_id: release.id.clone(),
        project_id: release.project_id.clone(),
        version: release
            .display_version
            .clone()
            .unwrap_or_else(|| release.name.clone()),
        build: build.map(|b| b.version.clone()).unwrap_or_default(),
        state: release.status.as_str().to_string(),
        installable: installable(release.status),
        for_testing: release.status == ReleaseStatus::AwaitingOwner,
        page_url: page,
        install_url,
        site: None,
        computer,
        // Starting `tailscale serve` is DevOps' job on this computer today.
        can_start_site: false,
        devices: phone_version::install_devices(app)?,
        app_title: app
            .cfg
            .releases
            .ios_title
            .clone()
            .unwrap_or_else(|| "The Hermes".to_string()),
        approved_at: approved_at(app, release)?.map(timestamp),
    })
}

/// When the owner approved the package, if they have.
fn approved_at(
    app: &AppState,
    release: &Release,
) -> anyhow::Result<Option<chrono::DateTime<chrono::Utc>>> {
    if !matches!(
        release.status,
        ReleaseStatus::Approved | ReleaseStatus::Deploying | ReleaseStatus::Deployed
    ) {
        return Ok(None);
    }
    let Some(id) = &release.decision_id else {
        return Ok(None);
    };
    Ok(app
        .db
        .get_decision(id)?
        .filter(|d| d.state != DecisionState::Held)
        .and_then(|d| d.ruling)
        .map(|r| r.answered_at))
}

/// The offer for one device: what its notification says and opens.
pub fn offer(info: &ReleaseInstall, device: &InstallDevice) -> InstallOffer {
    let name = if info.build.is_empty() || info.build == info.version {
        info.version.clone()
    } else {
        format!("{} ({})", info.version, info.build)
    };
    let body = if info.for_testing {
        "Ready to test. Tap to install.".to_string()
    } else if let Some(at) = info.approved_at.as_ref().and_then(|t| {
        chrono::DateTime::from_timestamp(t.seconds, 0).map(|at| at.format("%-d %b %Y"))
    }) {
        format!("Approved on {at}. Tap to install.")
    } else {
        "Tap to install.".to_string()
    };
    InstallOffer {
        release_id: info.release_id.clone(),
        project_id: info.project_id.clone(),
        device_id: device.device_id.clone(),
        device_name: device.name.clone(),
        version: info.version.clone(),
        build: info.build.clone(),
        install_url: info.install_url.clone(),
        page_url: info.page_url.clone(),
        title: format!("{} {name} is ready to install", info.app_title),
        body,
        for_testing: info.for_testing,
        sent_at: Some(timestamp(bus::now())),
        delivered: device.connected,
    }
}

fn offer_key(device_id: &str) -> String {
    format!("install_offer:{device_id}")
}

/// Sends a device the install link: keeps it as the device's waiting offer
/// and pushes it to the device's connections.
pub fn send(app: &AppState, release: &Release, device_id: &str) -> anyhow::Result<InstallOffer> {
    let info = info(app, release)?;
    if !info.installable {
        return Err(conflict(format!(
            "release {} is {}; it has no install link to send",
            release.name, info.state
        )));
    }
    if info.install_url.is_empty() {
        return Err(conflict(format!(
            "release {} has no iPhone build this computer serves as built; DevOps publishes it \
             after building",
            release.name
        )));
    }
    let device = info
        .devices
        .iter()
        .find(|d| d.device_id == device_id)
        .ok_or_else(|| not_found(format!("no paired device {device_id}")))?;
    let offer = offer(&info, device);
    app.db
        .set_meta(&offer_key(device_id), &serde_json::to_string(&offer)?)?;
    app.events.push(crate::events::Push::InstallOffer {
        device_id: device_id.to_string(),
        offer: Box::new(offer.clone()),
    });
    Ok(offer)
}

/// The device's waiting offer, if any.
pub fn waiting(app: &AppState, device_id: &str) -> anyhow::Result<Option<InstallOffer>> {
    let stored = app.db.get_meta(&offer_key(device_id))?;
    Ok(stored
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str(&s).ok()))
}

/// Not now: drops the device's offer for that release.
pub fn dismiss(app: &AppState, device_id: &str, release_id: &str) -> anyhow::Result<bool> {
    if release_id.trim().is_empty() {
        return Err(invalid("'release_id' is required"));
    }
    match waiting(app, device_id)? {
        Some(offer) if offer.release_id == release_id => {
            app.db.delete_meta(&offer_key(device_id))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// The devices holding a live connection to this daemon, counted per id.
#[derive(Default)]
pub struct LiveDevices(Mutex<HashMap<String, usize>>);

impl LiveDevices {
    pub fn connected(&self, device_id: &str) -> bool {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(device_id).is_some_and(|n| *n > 0)
    }

    fn change(&self, device_id: &str, up: bool) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let n = map.entry(device_id.to_string()).or_default();
        *n = if up { *n + 1 } else { n.saturating_sub(1) };
        if *n == 0 {
            map.remove(device_id);
        }
    }
}

/// A device's connection, counted while it lives.
pub struct Presence {
    app: Arc<AppState>,
    device_id: String,
}

impl Presence {
    pub fn enter(app: &Arc<AppState>, device_id: &str) -> Self {
        app.live_devices.change(device_id, true);
        Presence {
            app: app.clone(),
            device_id: device_id.to_string(),
        }
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        self.app.live_devices.change(&self.device_id, false);
    }
}

#[cfg(test)]
#[path = "phone_tests.rs"]
mod tests;
