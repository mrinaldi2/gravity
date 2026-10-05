//! `board_import` for the owner (B6), as `hermesd board import` sends it:
//! the backlog's text, the project by id or name (left out, the one board
//! on this computer) and `dry_run`. Needs `approve`: the plan makes the
//! import the owner's (H-020 §1.5).

use serde_json::{json, Value};

use super::Conn;
use crate::actor::Actor;
use crate::app::AppState;
use crate::board::import::service::{run, summary};

impl Conn {
    pub(super) fn board_import(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let markdown = Self::str_field(req, "markdown")?.to_string();
        let project = req
            .get("project")
            .and_then(Value::as_str)
            .map(str::to_string);
        let dry_run = req.get("dry_run").and_then(Value::as_bool).unwrap_or(false);
        let device = self.device_id.clone();
        self.blocking(req_id, move |app| {
            let project_id = project_with_board(app, project.as_deref())?;
            let actor = device.as_deref().map_or(Actor::User, Actor::Device);
            let report = run(app, &project_id, &markdown, dry_run, &actor)?;
            Ok(json!({
                "type": "board_imported",
                "summary": summary(&report),
                "report": report,
            }))
        });
        Ok(())
    }
}

/// The named project, or the only one with a board on this computer.
fn project_with_board(app: &AppState, name_or_id: Option<&str>) -> anyhow::Result<String> {
    let projects = app.db.list_projects()?;
    let mut with_board = Vec::new();
    for p in projects.into_iter().filter(|p| p.deleted_at.is_none()) {
        if app.db.board_settings(&p.id)?.is_some() {
            with_board.push(p);
        }
    }
    let found: Vec<_> = match name_or_id {
        Some(n) => with_board
            .into_iter()
            .filter(|p| p.id == n || p.name.eq_ignore_ascii_case(n))
            .collect(),
        None => with_board,
    };
    match found.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => anyhow::bail!(
            "no board here{}; the import runs on the board's home computer",
            name_or_id
                .map(|n| format!(" for project '{n}'"))
                .unwrap_or_default()
        ),
        _ => anyhow::bail!("several projects have a board here; name one with --project"),
    }
}
