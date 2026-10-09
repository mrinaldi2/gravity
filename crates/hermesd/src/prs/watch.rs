//! Every few minutes, looks for PR branches that moved with no report
//! (H-261 §1.2), so `moved_unreported` shows without anyone reading the PR.

use std::sync::Arc;
use std::time::Duration;

use crate::app::AppState;

const EVERY: Duration = Duration::from_secs(300);

pub fn spawn(app: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            let app = app.clone();
            let _ = tokio::task::spawn_blocking(move || step(&app)).await;
        }
    });
}

/// One pass over the projects with an open PR.
pub fn step(app: &AppState) {
    let projects = match app.db.board_read(|t| t.projects_with_live_prs()) {
        Ok(projects) => projects,
        Err(e) => {
            tracing::warn!(error = %e, "couldn't list projects with open PRs");
            return;
        }
    };
    for project in projects {
        super::look(app, &project);
    }
}
