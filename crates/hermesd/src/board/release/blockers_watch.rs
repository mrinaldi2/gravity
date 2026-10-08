//! Keeping "Waiting for you" live (H-247, UX-048 §6c): every few seconds the
//! open releases' owner blockers are read again, and a release whose list or
//! work card changed is pushed as `release_updated`. A release with no work
//! card gets the card named for it (`REL-<version>`), logged once (§6d).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::blockers::owner_blockers;
use crate::app::AppState;
use crate::events::Push;

/// How often the open releases are read again.
const EVERY: Duration = Duration::from_secs(5);

/// What a release's line shows: its work card and its blockers by kind and id.
type Seen = (Option<String>, Vec<(&'static str, String)>);

pub fn spawn(app: Arc<AppState>) {
    tokio::spawn(async move {
        let mut seen: HashMap<String, Seen> = HashMap::new();
        let mut tick = tokio::time::interval(EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(e) = step(&app, &mut seen) {
                tracing::warn!(error = %e, "release blockers watch failed");
            }
        }
    });
}

/// One pass: backfill, then push the releases whose line changed. The first
/// sight of a release only records it.
pub fn step(app: &AppState, seen: &mut HashMap<String, Seen>) -> anyhow::Result<()> {
    let open = app.db.board_read(|t| t.open_release_ids())?;
    let mut now = HashMap::with_capacity(open.len());
    for (id, project_id) in open {
        backfill(app, &id)?;
        let Some(release) = app.db.board_read(|t| t.release(&id))? else {
            continue;
        };
        let blockers = owner_blockers(app, &release)?
            .into_iter()
            .map(|b| (b.kind, b.id))
            .collect();
        let line = (release.work_item_id.clone(), blockers);
        if seen.get(&id).is_some_and(|before| before != &line) {
            app.events.push(Push::ReleaseUpdated {
                project_id,
                release_id: id.clone(),
            });
        }
        now.insert(id, line);
    }
    *seen = now;
    Ok(())
}

/// A release without a work card gets the card named for its version.
fn backfill(app: &AppState, release_id: &str) -> anyhow::Result<()> {
    app.db.board_tx(|t| {
        let Some(release) = t.release(release_id)? else {
            return Ok(());
        };
        if release.work_item_id.is_some() {
            return Ok(());
        }
        let version = release.display_version.as_deref().unwrap_or(&release.name);
        if let Some(card) = t.rel_card_named(&release.project_id, version)? {
            t.set_release_work_item(release_id, &card, "daemon:backfill")?;
            tracing::info!(
                release_id,
                card,
                version,
                "release work card found by its REL title"
            );
        }
        Ok(())
    })
}
