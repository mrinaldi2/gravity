//! A computer's part of a project (H-128 §2.3): the attention rows it owns,
//! its own bots at work, and, on the board's home, the current release and
//! the latest meeting summary. The overview builds its own part the same
//! way it asks a peer for one, so both sides count alike.

use bus::contract::home::{project_attention::Part, ReleaseBrief, SummaryBrief};
use bus::BotState;
use serde_json::Value;

use crate::app::AppState;
use crate::attention::{self, cut, timestamp, Scope};
use crate::board::model::ColumnCategory;
use crate::board::release::model::{Release, ReleaseStatus};

/// The longest meeting summary a row carries.
const SUMMARY_MAX: usize = 200;

/// This computer's part of the project, in its own ids.
pub fn part(app: &AppState, project_id: &str) -> anyhow::Result<Part> {
    let scope = Scope::here(app, project_id);
    let mut rows: Vec<_> = attention::rows(app, project_id, scope, &|_| Ok(Value::Null))?
        .into_iter()
        .map(|b| b.row)
        .collect();
    attention::sort(&mut rows);
    let local = attention::summary(&rows);
    let states: Vec<BotState> = app
        .db
        .list_bots(Some(project_id))?
        .iter()
        .filter(|bot| !bot.is_linked())
        .map(|bot| app.supervisor.state(&bot.id).0)
        .collect();
    let bots_working = states
        .iter()
        .filter(|s| matches!(s, BotState::Working | BotState::Starting))
        .count();
    let bots_waiting = states
        .iter()
        .filter(|s| **s == BotState::WaitingForUser)
        .count();
    let (current_release, latest_summary) = if scope.home {
        (
            current_release(app, project_id)?,
            app.db
                .latest_meeting_summary(project_id)?
                .map(|s| SummaryBrief {
                    meeting_id: s.meeting_id,
                    text: cut(&s.text, SUMMARY_MAX),
                    at: Some(timestamp(s.at)),
                }),
        )
    } else {
        (None, None)
    };
    Ok(Part {
        project_id: project_id.to_string(),
        local: Some(local),
        rows,
        bots_working: u32::try_from(bots_working).unwrap_or(u32::MAX),
        is_home: scope.home,
        current_release,
        latest_summary,
        last_activity_at: app.db.project_last_activity(project_id)?.map(timestamp),
        pinned: app.db.project_pinned(project_id)?,
        bots_waiting: u32::try_from(bots_waiting).unwrap_or(u32::MAX),
    })
}

/// The newest release not over, else the last one deployed (H-128 §2.1).
fn current_release(app: &AppState, project_id: &str) -> anyhow::Result<Option<ReleaseBrief>> {
    let releases = app.db.board_read(|t| t.releases(project_id))?;
    let Some(release) = releases.iter().find(|r| !r.status.is_closed()).or_else(|| {
        releases
            .iter()
            .find(|r| r.status == ReleaseStatus::Deployed)
    }) else {
        return Ok(None);
    };
    let done_columns: Vec<String> = app
        .db
        .board_columns(project_id)?
        .into_iter()
        .filter(|c| c.category == ColumnCategory::Done)
        .map(|c| c.key)
        .collect();
    let cards = app.db.board_cards(project_id)?;
    let done = release
        .items
        .iter()
        .filter(|item| {
            cards
                .iter()
                .any(|c| c.id == item.item_id && done_columns.contains(&c.column_key))
        })
        .count();
    Ok(Some(brief(release, done)))
}

fn brief(release: &Release, items_done: usize) -> ReleaseBrief {
    let deployed_at = release
        .deployments
        .iter()
        .filter_map(|d| d.at)
        .max()
        .filter(|_| release.status == ReleaseStatus::Deployed);
    ReleaseBrief {
        release_id: release.id.clone(),
        version: release
            .display_version
            .clone()
            .unwrap_or_else(|| release.name.clone()),
        state: release.status.as_str().to_string(),
        awaiting_owner: release.status == ReleaseStatus::AwaitingOwner,
        items_total: u32::try_from(release.items.len()).unwrap_or(u32::MAX),
        items_done: u32::try_from(items_done).unwrap_or(u32::MAX),
        items_ready: u32::try_from(release.plan.iter().filter(|p| p.ready).count())
            .unwrap_or(u32::MAX),
        deployed_at: deployed_at.map(timestamp),
    }
}
