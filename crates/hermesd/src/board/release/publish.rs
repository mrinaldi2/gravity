//! Publishing a build to the tailnet and checking it at install (H-020 §6.6).
//!
//! DevOps with the Publish extra stages a build file with `release_publish`
//! (what `hermesd release publish` calls): it is copied into the served
//! directory, hashed, and attached to the package with its HTTPS `url` and
//! `install_url`. `install_release` re-hashes every served build against the
//! frozen sha256 and refuses on a mismatch, pausing the rollout.

use std::path::Path;
use std::sync::Arc;

use bus::PermissionExtra;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::messaging::{self, daemon_sender, Dm};

use super::assemble::attach_build;
use super::confine::{self, SERVING_OFF};
use super::lifecycle::tell_installers;
use super::model::{Release, ReleaseBuild, ReleaseStatus};
use super::serve::{self, Staged};
use super::{load, Caller};

pub struct Publish<'a> {
    pub release_id: &'a str,
    /// An absolute path under an allowed root.
    pub file: &'a str,
    pub platform: Option<&'a str>,
    pub version: Option<&'a str>,
    /// iOS only: the app's bundle id for the manifest.
    pub bundle_id: Option<&'a str>,
    /// The git commit it was built from (ARCH-R52 M1).
    pub source_commit: Option<&'a str>,
}

/// Stage `req.file` in the served directory and attach it as the package's
/// build for its platform; returns the package and what was published.
pub fn publish(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    req: &Publish<'_>,
) -> anyhow::Result<(Release, Value)> {
    me.require(Role::Devops, "publish a build")?;
    if !app
        .db
        .bot_permission_extras(&me.bot.id)?
        .contains(&PermissionExtra::Publish)
    {
        return Err(forbidden(
            "publishing builds needs the Publish permission extra; ask the owner to grant it",
        ));
    }
    let project = app
        .db
        .get_project(&me.bot.project_id)?
        .ok_or_else(|| not_found("no such project"))?;
    let release = app
        .db
        .board_read(|t| load(t, &project.id, req.release_id))?;
    super::plan::builds_wait(&release)?;
    if !release.status.is_assembling() {
        return Err(conflict(format!(
            "release {} is {}; builds change only before it is submitted",
            release.name,
            release.status.as_str()
        )));
    }
    let cfg = &app.cfg;
    let base = serve::base_url(cfg)?;
    let artifacts = crate::paths::artifacts_dir(cfg, &project.dir_name);
    let src = confine::source(cfg, &artifacts, Path::new(req.file.trim()))?;
    let name = src
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let given = |v: Option<&str>| v.map(str::trim).filter(|v| !v.is_empty()).map(String::from);
    let platform = given(req.platform)
        .or_else(|| serve::platform_for(&name).map(String::from))
        .ok_or_else(|| {
            invalid(format!(
                "can't tell the platform of {name}; pass 'platform'"
            ))
        })?;
    let version = given(req.version)
        .or_else(|| {
            let attached = release.builds.iter().find(|b| b.platform == platform);
            attached.map(|b| b.version.clone())
        })
        .ok_or_else(|| {
            invalid(format!(
                "'version' is required: {platform} has no build yet"
            ))
        })?;
    let ios = name.to_ascii_lowercase().ends_with(".ipa");
    let bundle_id = given(req.bundle_id).or_else(|| given(cfg.releases.ios_bundle_id.as_deref()));
    if ios && bundle_id.is_none() {
        return Err(invalid(
            "an .ipa needs 'bundle_id' for its manifest, or [releases] ios_bundle_id configured",
        ));
    }

    // The owner hears of it, not only the bot that tried (H-100).
    let dirs = confine::prepare(cfg).inspect_err(|e| {
        app.events.push(crate::events::Push::notice(
            "error",
            SERVING_OFF,
            format!("{e:#}"),
        ));
    })?;
    let staged = serve::stage(&dirs, &base, &release.id, &platform, src)?;
    let install_url = match bundle_id.filter(|_| ios) {
        Some(bundle) => {
            let title = cfg.releases.ios_title.as_deref().unwrap_or("The Hermes");
            let manifest = serve::write_manifest(&dirs, &staged, &bundle, &version, title)?;
            let link = format!("itms-services://?action=download-manifest&url={manifest}");
            // The page Copy link and the QR code open (H-229). A page left
            // from an earlier file is kept: published files never change.
            let dir = staged.path.parent().expect("staged in a directory");
            let page = super::phone::page_html(title, &version, &link);
            if let Err(e) = serve::write_page(&dirs, dir, &page) {
                tracing::warn!(release = %release.id, "the install page wasn't written: {e:#}");
            }
            link
        }
        None => staged.url.clone(),
    };
    let build = ReleaseBuild {
        platform,
        version,
        artifact: staged.path.display().to_string(),
        url: Some(staged.url.clone()),
        install_url: Some(install_url.clone()),
        sha256: staged.sha256.clone(),
        built_at: bus::now(),
        source_commit: given(req.source_commit),
    };
    let release = attach_build(app, me, &release.id, &build)?;
    Ok((release, published(&staged, &build, &install_url)))
}

fn published(staged: &Staged, build: &ReleaseBuild, install_url: &str) -> Value {
    json!({
        "platform": build.platform, "version": build.version,
        "path": staged.path.display().to_string(), "url": staged.url,
        "install_url": install_url, "sha256": staged.sha256,
        "already_published": !staged.fresh,
    })
}

/// Every build this daemon serves, re-hashed against the frozen sha256
/// before a tester may install it. On a mismatch the install is refused, a
/// rollout in progress is paused, and DevOps is told. Returns, per build,
/// whether it was checked here (a build attached by path is the tester's to
/// check).
pub fn verify_served(app: &Arc<AppState>, release: &Release) -> anyhow::Result<Vec<bool>> {
    let mut checked = Vec::new();
    for b in &release.builds {
        let Some(path) = serve::served_file(&app.cfg, &b.artifact) else {
            checked.push(false);
            continue;
        };
        if !serve::matches(&path, &b.sha256) {
            let why = format!(
                "the served {} build ({}) no longer matches its sha256 {}",
                b.platform,
                path.display(),
                b.sha256
            );
            flag(app, release, &why)?;
            return Err(conflict(format!(
                "release {} can't be installed: {why}; DevOps has been told",
                release.name
            )));
        }
        checked.push(true);
    }
    Ok(checked)
}

/// Pause a rollout in progress over a bad build, and tell DevOps.
fn flag(app: &Arc<AppState>, release: &Release, why: &str) -> anyhow::Result<()> {
    let reason = format!("{why}; republish it in a new release");
    let paused = app.db.board_tx(|t| {
        let now = t.release(&release.id)?.expect("loaded");
        if !matches!(
            now.status,
            ReleaseStatus::Deploying | ReleaseStatus::PartiallyDeployed
        ) {
            return Ok(None);
        }
        t.set_release_paused(&release.id, Some(&reason))?;
        t.set_release_status(&release.id, ReleaseStatus::Paused)?;
        t.release(&release.id)
    })?;
    let sender = daemon_sender();
    if let Some(paused) = &paused {
        let note = format!(
            "Release {} is paused: {reason}. Don't install it until DevOps resumes the rollout.",
            release.name
        );
        tell_installers(app, paused, &sender, &note)?;
    }
    let note = format!(
        "Release {} (release_id {}): an install was refused because {why}.{}",
        release.name,
        release.id,
        if paused.is_some() {
            " Its rollout is paused."
        } else {
            ""
        }
    );
    let told = messaging::send_dm(
        &app.db,
        &app.events,
        Dm::new(&release.created_by, &sender, bus::MessageKind::Note, &note),
    );
    if let Err(e) = told {
        tracing::warn!(release = %release.id, "couldn't tell DevOps about a bad build: {e:#}");
    }
    Ok(())
}
