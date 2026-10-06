//! At boot, after an install whose rollback failed too (ARCH-R52 S3): the
//! install job left a marker and brought a daemon up from `<home>/bin`. The
//! owner gets a notice, and a Run card that installs the service from the
//! app in Applications, from a fixed template: the marker's app path is used
//! only when it is a plain `/Applications/<name>.app`.

use std::sync::Arc;

use serde_json::Value;

use crate::app::AppState;
use crate::events::Push;
use crate::owner_action::model::{Proposal, Shell};
use crate::owner_action::{self, writable_roots};

use super::apply::FAILED_MARKER;

/// A path the Run card may name: `/Applications/<name>.app`, nothing else.
pub(crate) fn plain_app(path: &str) -> Option<&str> {
    let name = path.strip_prefix("/Applications/")?.strip_suffix(".app")?;
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ._-".contains(c));
    plain.then_some(path)
}

/// Tells the owner, once, and files the Run card.
pub fn report(app: &Arc<AppState>) {
    let path = app.cfg.home.join("run").join(FAILED_MARKER);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let _ = std::fs::remove_file(&path);
    let marker: Value = serde_json::from_str(&text).unwrap_or_default();
    let release = marker["release"].as_str().unwrap_or("a release");
    let why = marker["why"].as_str().unwrap_or("unknown");
    let body = format!(
        "The install of {release} failed, and putting the earlier app back failed too: {why}. \
         The Hermes service was started from the last installed program. A command that \
         installs the service from the app in Applications is waiting for you to run."
    );
    app.events.push(Push::notice(
        "error",
        "An install and its rollback both failed",
        &body,
    ));
    let Some(bundle) = marker["app"].as_str().and_then(plain_app) else {
        return;
    };
    let project = app
        .db
        .board_read(|t| t.release(release))
        .ok()
        .flatten()
        .map(|r| (r.project_id, r.decision_id));
    let Some((project_id, decision_id)) = project else {
        return;
    };
    let proposal = Proposal {
        project_id,
        proposed_by: "daemon".to_string(),
        item_id: None,
        decision_id,
        target_machine: app.db.daemon_id().unwrap_or_default(),
        shell: Shell::Bash,
        cwd: app.cfg.user_home.display().to_string(),
        content: format!("'{bundle}/Contents/MacOS/hermesd' service install"),
        pinned_files: Vec::new(),
        reason: format!(
            "The install of {release} and its rollback failed. This reinstalls the Hermes \
             service from {bundle}, the app now in Applications."
        ),
        timeout_s: 300,
    };
    if let Err(e) = owner_action::store(app, proposal, "daemon", writable_roots(app)) {
        tracing::warn!(error = %e, "could not file the rollback's Run card");
    }
}

#[cfg(test)]
mod tests {
    use super::plain_app;

    #[test]
    fn the_card_names_only_a_plain_app_in_applications() {
        assert!(plain_app("/Applications/The Hermes.app").is_some());
        assert!(plain_app("/Applications/x'; rm -rf ~; '.app").is_none());
        assert!(plain_app("/tmp/The Hermes.app").is_none());
        assert!(plain_app("/Applications/../x.app").is_none());
        assert!(plain_app("/Applications/a/b.app").is_none());
    }
}
