//! Installing an iOS package on a paired iPhone or iPad (H-229, UX-043):
//! the HTTPS install page beside the build, whether the build site answers,
//! and the offer a device gets when the owner sends it the link.
//!
//! An offer is the device's one waiting install link, kept in `meta` under
//! `install_offer:<device id>` until the device dismisses it or a newer one
//! replaces it, so a phone that wasn't connected sees it when it next opens.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bus::contract::home::{InstallDevice, InstallOffer, ReleaseInstall, SiteCheck};
use bus::DecisionState;

use super::model::{Release, ReleaseBuild, ReleaseStatus};
use super::{confine, machines, serve};
use crate::app::AppState;
use crate::attention::timestamp;
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

/// The HTTPS install page for a build's url: the url itself when it is a
/// page, otherwise `index.html` in the same folder. None when it isn't HTTPS.
pub fn page_url(build_url: &str) -> Option<String> {
    let url = build_url.trim();
    if !url.starts_with("https://") {
        return None;
    }
    if url.ends_with(".html") {
        return Some(url.to_string());
    }
    let (dir, _) = url.rsplit_once('/')?;
    (dir.len() > "https://".len()).then(|| format!("{dir}/{PAGE}"))
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

/// Writes the install page beside a build this daemon serves, if it isn't
/// there yet: builds published before H-229 have none.
pub fn ensure_page(app: &AppState, build: &ReleaseBuild) -> anyhow::Result<()> {
    let (Some(path), Some(install_url)) = (
        serve::served_file(&app.cfg, &build.artifact),
        build.install_url.as_deref(),
    ) else {
        return Ok(());
    };
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    if dir.join(PAGE).exists() {
        return Ok(());
    }
    let dirs = confine::prepare(&app.cfg)?;
    let title = app
        .cfg
        .releases
        .ios_title
        .as_deref()
        .unwrap_or("The Hermes");
    serve::write_page(&dirs, dir, &page_html(title, &build.version, install_url))
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
    if let Some(b) = build {
        // A missing page only costs the Copy link its target; say so in the
        // log rather than fail the whole answer.
        if let Err(e) = ensure_page(app, b) {
            tracing::warn!(release = %release.id, "couldn't write the install page: {e:#}");
        }
    }
    let install_url = build
        .and_then(|b| b.install_url.clone())
        .unwrap_or_default();
    let page = build
        .and_then(|b| b.url.as_deref())
        .and_then(page_url)
        .filter(|_| !install_url.is_empty())
        .unwrap_or_default();
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
        devices: devices(app)?,
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

/// The paired devices that may be sent a link, last seen first.
fn devices(app: &AppState) -> anyhow::Result<Vec<InstallDevice>> {
    let mut list: Vec<_> = app
        .db
        .list_devices()?
        .into_iter()
        .filter(|d| d.revoked_at.is_none())
        .collect();
    list.sort_by_key(|d| std::cmp::Reverse(d.last_seen_at));
    Ok(list
        .into_iter()
        .map(|d| InstallDevice {
            connected: app.live_devices.connected(&d.id),
            last_seen_at: d.last_seen_at.map(timestamp),
            device_id: d.id,
            name: d.name,
            app_version: String::new(),
        })
        .collect())
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
            "release {} has no iPhone build to install yet; DevOps publishes it after building",
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
