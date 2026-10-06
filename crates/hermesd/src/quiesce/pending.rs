//! "A release install is pending on this computer" (H-166), for bots.
//!
//! A colima VM started for a VR run held the home through its share and
//! blocked the 0.17.0 install. While an install is under way here (a bot on
//! this computer holds an open deploy or rollback task, or a pause is open)
//! the daemon keeps `<home>/run/install-pending.json` fresh, and the guard
//! refuses to start a VM or a VR run (`guard::commands`). The file is
//! rewritten every few seconds; one the daemon stopped refreshing counts as
//! gone, so a crash never blocks VMs for good.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};

use crate::app::AppState;

/// How often the daemon looks.
const REFRESH: Duration = Duration::from_secs(15);
/// A file older than this is a daemon that stopped refreshing it.
const FRESH: Duration = Duration::from_secs(120);

pub fn path(home: &Path) -> PathBuf {
    home.join("run").join("install-pending.json")
}

/// What is pending here, if anything.
pub fn current(app: &AppState) -> anyhow::Result<Option<Value>> {
    if let Some(q) = app.db.open_quiesce()? {
        return Ok(Some(json!({ "why": q.reason, "release_id": q.release_id })));
    }
    Ok(app.db.install_task_here()?.map(|(release_id, bot)| {
        json!({ "why": format!("{bot} is installing a release"), "release_id": release_id })
    }))
}

/// Write the file, or remove it when nothing is pending.
pub fn refresh(app: &AppState) -> anyhow::Result<()> {
    let file = path(&app.cfg.home);
    match current(app)? {
        Some(pending) => {
            if let Some(run) = file.parent() {
                std::fs::create_dir_all(run)?;
            }
            crate::paths::atomic_write_json(&file, &pending)
        }
        None => match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        },
    }
}

/// Keeps the file current for as long as the daemon runs.
pub async fn watch(app: Arc<AppState>) {
    let mut tick = tokio::time::interval(REFRESH);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let app = app.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = refresh(&app) {
                tracing::debug!(error = %e, "install-pending flag not refreshed");
            }
        })
        .await;
    }
}

/// Why a bot may not start a VM or a VR run now, read by the guard.
pub fn blocking(home: &Path) -> Option<String> {
    let file = path(home);
    let age = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|at| SystemTime::now().duration_since(at).ok())?;
    if age > FRESH {
        return None;
    }
    let text = std::fs::read_to_string(&file).ok()?;
    let why = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v["why"].as_str().map(str::to_string))
        .unwrap_or_else(|| "a release install".to_string());
    Some(format!(
        "a release install is pending on this computer ({why}); don't start a VM or a VR \
         run until it's done, since one holding the home blocks the install. Try again later"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_file_blocks_and_a_stale_or_missing_one_does_not() {
        let dir = tempfile::tempdir().expect("dir");
        assert_eq!(blocking(dir.path()), None);
        let file = path(dir.path());
        std::fs::create_dir_all(file.parent().expect("run")).expect("run dir");
        std::fs::write(&file, r#"{"why":"install of 0.17.1"}"#).expect("write");
        let why = blocking(dir.path()).expect("blocks");
        assert!(why.contains("install of 0.17.1"), "{why}");
        let old = SystemTime::now() - FRESH - Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .and_then(|f| f.set_modified(old))
            .expect("age it");
        assert_eq!(blocking(dir.path()), None, "a daemon that stopped refreshing");
    }
}
