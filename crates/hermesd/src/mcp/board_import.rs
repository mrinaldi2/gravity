//! `board_import` for the lead (B6): the backlog file comes from the
//! project's artifacts and nowhere else.

use std::sync::Arc;

use bus::contract::board as c;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::import::service::{run, summary};

use super::board::{out, Me};

pub(super) fn call(app: &Arc<AppState>, me: &Me, req: c::BoardImport) -> anyhow::Result<Value> {
    let project = app
        .db
        .get_project(&me.bot.project_id)?
        .ok_or_else(|| anyhow::anyhow!("no project {}", me.bot.project_id))?;
    let root = crate::chat::files::artifacts_dir(app, &project);
    let path = req.path.as_deref().unwrap_or("backlog.md");
    let resolved = root
        .join(path)
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("no file {path} in the project's artifacts"))?;
    anyhow::ensure!(
        root.canonicalize()
            .is_ok_and(|root| resolved.starts_with(root)),
        "{path} is outside the project's artifacts"
    );
    let markdown = std::fs::read_to_string(&resolved)
        .map_err(|e| anyhow::anyhow!("can't read {path}: {e}"))?;
    let report = run(
        app,
        &project.id,
        &markdown,
        req.dry_run.unwrap_or(false),
        &me.actor(),
    )?;
    Ok(json!({ "summary": summary(&report), "report": out(&report)? }))
}
