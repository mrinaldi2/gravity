//! The daemon removing what retired workers left on disk (H-109), once their
//! unpushed commits are bundled (ARCH-R40). See `scratch` for what goes.

use std::path::Path;
use std::sync::Arc;

use super::{scratch, Workers};
use crate::app::AppState;

/// Removes what retired workers left on disk: every retired worker on the
/// first pass after start, then the newly retired and any that failed
/// before. Then the shared Cargo target, once no worker is left. Awaited, so
/// a worker placed in the same pass never builds into a target being
/// deleted.
pub(super) async fn clean_retired(app: &Arc<AppState>) {
    let mut ids: Vec<String> = Workers::set(&app.workers.unclean).drain().collect();
    if !app
        .workers
        .swept
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        match app.db.retired_worker_workspaces() {
            Ok(all) => ids.extend(all.into_iter().map(|(id, _)| id)),
            Err(error) => tracing::warn!(%error, "listing retired workers failed"),
        }
    }
    let app = app.clone();
    let cleaned = tokio::task::spawn_blocking(move || {
        for id in ids {
            match app.db.get_bot(&id) {
                Ok(Some(bot)) => clean_one(&app, &bot),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(bot_id = %id, %error, "reading a retired worker failed")
                }
            }
        }
        if let Err(error) = scratch::clean_shared_target(&app) {
            tracing::warn!(%error, "removing the workers' shared target failed");
        }
    })
    .await;
    if let Err(error) = cleaned {
        tracing::warn!(%error, "cleaning up after workers failed");
    }
}

fn clean_one(app: &Arc<AppState>, bot: &bus::Bot) {
    let workspace = Path::new(&bot.workspace_path);
    if !scratch::has_leftovers(workspace) {
        return;
    }
    if let Some(reason) = scratch::refuse_cleaning(&app.cfg.projects_dir(), bot) {
        tracing::error!(bot_id = %bot.id, workspace = %bot.workspace_path, %reason, "refusing to delete a worker's folders");
        return;
    }
    match scratch::clean_worker(workspace) {
        Ok(cleaned) => report(app, bot, &cleaned),
        Err(error) => {
            tracing::warn!(bot_id = %bot.id, %error, "removing a retired worker's clone failed; retrying later");
            Workers::set(&app.workers.unclean).insert(bot.id.clone());
        }
    }
}

/// Tells the worker's parent, as the daemon (the worker is archived), where
/// commits no remote had went, or which repositories stayed and why.
fn report(app: &Arc<AppState>, bot: &bus::Bot, cleaned: &scratch::Cleaned) {
    for bundle in &cleaned.bundles {
        let body = format!(
            "{} left commits no remote has; they are saved in the bundle {} on its machine. \
             `git fetch <bundle>` gets them.",
            bot.name,
            bundle.display()
        );
        super::notify_parent(app, bot, &crate::messaging::daemon_sender(), &body);
    }
    for (repo, error) in &cleaned.kept {
        tracing::warn!(bot_id = %bot.id, repo = %repo.display(), %error, "bundling a retired worker's repository failed; kept");
        let body = format!(
            "{}: {} could not be checked or bundled for commits no remote has ({error}), \
             so it is kept on its machine instead of being deleted.",
            bot.name,
            repo.display()
        );
        super::notify_parent(app, bot, &crate::messaging::daemon_sender(), &body);
    }
}
