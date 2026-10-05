//! `metrics_get {project_id, range}` (B11, read): the dashboard's Flow
//! widget. `range` is `week` (7 days, the default) or `4w` (28 days).
//!
//! The history lives where the board does. Off its home, the daemon asks the
//! home (peer request `metrics_get`) and answers once it has; when the home
//! can't be reached, `metrics` is null and `note` says where to look.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration, Utc};
use serde_json::{json, Value};

use super::Conn;
use crate::app::AppState;
use crate::board::metrics::{compute, Column};

/// Days a range covers.
fn range_days(range: Option<&str>) -> anyhow::Result<i64> {
    match range.unwrap_or("week") {
        "week" => Ok(7),
        "4w" => Ok(28),
        other => anyhow::bail!("'range' must be week or 4w, not '{other}'"),
    }
}

/// The metrics of a project whose board is on this computer, as JSON.
pub(crate) fn home_metrics(app: &AppState, project_id: &str, days: i64) -> anyhow::Result<Value> {
    let db = &app.db;
    let columns: Vec<Column> = db
        .board_columns(project_id)?
        .into_iter()
        .map(|c| Column {
            key: c.key,
            name: c.name,
            category: c.category,
        })
        .collect();
    let titles: HashMap<String, String> = db
        .board_cards(project_id)?
        .into_iter()
        .map(|c| (c.id, c.title))
        .collect();
    let now = Utc::now();
    let flow = compute(&db.flow_entries(project_id)?, &columns, &titles, now, days);
    let mut value = serde_json::to_value(&flow)?;
    value["expired_tasks"] = json!(db.expired_tasks_since(project_id, now - Duration::days(days))?);
    value["days"] = json!(days);
    Ok(value)
}

impl Conn {
    pub(super) fn metrics_get(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = Self::str_field(req, "project_id")?.to_string();
        let days = range_days(req.get("range").and_then(Value::as_str))?;
        let app = &self.app;
        if app.db.board_settings(&project_id)?.is_some() {
            let metrics = home_metrics(app, &project_id, days)?;
            self.send(
                json!({ "type": "metrics", "req_id": req_id, "metrics": metrics, "note": null }),
            );
            return Ok(());
        }
        let Some(home) = app.board_mirror.home_peer(&project_id) else {
            self.send(
                json!({ "type": "metrics", "req_id": req_id, "metrics": null, "note": null }),
            );
            return Ok(());
        };
        self.answer_later(req_id, from_home(app.clone(), project_id, home, days));
        Ok(())
    }
}

async fn from_home(
    app: Arc<AppState>,
    project_id: String,
    home: String,
    days: i64,
) -> anyhow::Result<Value> {
    let frame = json!({ "type": "metrics_get", "project_id": project_id, "days": days });
    Ok(match app.peers.request(&home, frame).await {
        Ok(answer) => json!({ "type": "metrics", "metrics": answer["metrics"], "note": null }),
        Err(_) => {
            let name = crate::peer::board::home_name(&app, &home);
            json!({
                "type": "metrics", "metrics": null,
                "note": format!("Flow is kept on {name}, which can't be reached right now."),
            })
        }
    })
}
