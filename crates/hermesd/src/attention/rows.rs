//! The row builder (H-128 §1): one function per kind, each reading only
//! what this computer owns.

use std::collections::HashMap;

use bus::contract::home::{attention_row::Target, AttentionKind, AttentionRow, BotRef};
use bus::{BotState, Decision, DecisionState};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::legacy::{relayed_row, relayed_ruling};
use super::{decision_weight, row_id, timestamp, title, weight, Built};
use crate::app::AppState;
use crate::board::model::{ColumnCategory, Priority};
use crate::board::release::confine;
use crate::board::release::model::{Release, ReleaseStatus};
use crate::db::DecisionFilter;
use crate::decisions::authority::is_relayed;
use crate::owner_action::model::State as ActionState;

/// Which of a project's rows a computer builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scope {
    /// This computer's: prompts, Run cards, waiting bots, decisions, serving.
    pub local: bool,
    /// The board's: releases awaiting the owner and P0 cards.
    pub home: bool,
}

impl Scope {
    /// Everything this computer owns for the project.
    pub fn here(app: &AppState, project_id: &str) -> Self {
        Self {
            local: true,
            home: is_home(app, project_id),
        }
    }
}

/// Whether the project's board lives on this computer.
pub(crate) fn is_home(app: &AppState, project_id: &str) -> bool {
    let me = app.db.daemon_id().unwrap_or_default();
    app.db
        .board_settings(project_id)
        .ok()
        .flatten()
        .is_some_and(|s| s.home_daemon_id == me)
}

/// What one row is made of, before its id and weight are added.
struct Part {
    kind: AttentionKind,
    target_id: String,
    title: String,
    created_at: DateTime<Utc>,
    target: Option<Target>,
}

struct Builder<'a> {
    me: String,
    project_id: &'a str,
    out: Vec<Built>,
}

impl Builder<'_> {
    fn push(&mut self, part: Part, weight: u32, legacy: Option<Value>) -> &mut AttentionRow {
        self.out.push(Built {
            row: AttentionRow {
                id: row_id(part.kind, &self.me, &part.target_id),
                kind: part.kind as i32,
                daemon_id: self.me.clone(),
                project_id: self.project_id.to_string(),
                title: title(&part.title),
                created_at: Some(timestamp(part.created_at)),
                weight,
                target: part.target,
                ..AttentionRow::default()
            },
            legacy,
        });
        &mut self.out.last_mut().expect("just pushed").row
    }
}

/// The project's rows this computer owns within `scope`, in the order the
/// dashboard has always listed them; `release_json` renders a release for
/// the dashboard's shape of its row.
pub(crate) fn rows(
    app: &AppState,
    project_id: &str,
    scope: Scope,
    release_json: &dyn Fn(&Release) -> anyhow::Result<Value>,
) -> anyhow::Result<Vec<Built>> {
    let mut b = Builder {
        me: app.db.daemon_id()?,
        project_id,
        out: Vec::new(),
    };
    let releases = if scope.home {
        app.db.board_read(|t| t.releases(project_id))?
    } else {
        Vec::new()
    };
    if scope.local {
        serving_off(app, &mut b);
    }
    if scope.home {
        awaiting_releases(&releases, &mut b, release_json)?;
    }
    if scope.local {
        decisions(app, &releases, &mut b)?;
    }
    if scope.home {
        p0_items(app, &mut b)?;
    }
    if scope.local {
        owner_actions(app, &mut b)?;
        prompts_and_waiting(app, &mut b)?;
        owner_questions(app, &mut b)?;
    }
    Ok(b.out)
}

/// The served folder is refused, so no build can be published (H-100).
fn serving_off(app: &AppState, b: &mut Builder<'_>) {
    let Some(reason) = confine::serving_refused(&app.cfg) else {
        return;
    };
    let part = Part {
        kind: AttentionKind::ServingOff,
        target_id: b.project_id.to_string(),
        title: confine::SERVING_OFF.to_string(),
        created_at: app.overview.started_at(),
        target: None,
    };
    let legacy = json!({ "kind": "serving_off", "title": confine::SERVING_OFF, "reason": reason });
    b.push(part, weight(AttentionKind::ServingOff), Some(legacy));
}

fn awaiting_releases(
    releases: &[Release],
    b: &mut Builder<'_>,
    release_json: &dyn Fn(&Release) -> anyhow::Result<Value>,
) -> anyhow::Result<()> {
    for release in releases
        .iter()
        .filter(|r| r.status == ReleaseStatus::AwaitingOwner)
    {
        let version = release.display_version.as_deref().unwrap_or(&release.name);
        let part = Part {
            kind: AttentionKind::ReleaseAwaiting,
            target_id: release.id.clone(),
            title: format!("Release {version} waits for your ruling"),
            created_at: release.updated_at,
            target: Some(Target::ReleaseId(release.id.clone())),
        };
        let legacy = json!({ "kind": "release", "release": release_json(release)? });
        b.push(part, weight(AttentionKind::ReleaseAwaiting), Some(legacy));
    }
    Ok(())
}

/// Open decisions, and the rulings bots relayed as one row. A release's
/// decision is the release's row, not a second one.
fn decisions(app: &AppState, releases: &[Release], b: &mut Builder<'_>) -> anyhow::Result<()> {
    let release_decisions: Vec<&str> = releases
        .iter()
        .filter_map(|r| r.decision_id.as_deref())
        .collect();
    let decisions = project_decisions(app, b.project_id)?;
    let (relayed, open): (Vec<&Decision>, Vec<&Decision>) = decisions
        .iter()
        .filter(|d| !release_decisions.contains(&d.id.as_str()))
        .filter(|d| d.state == DecisionState::Open || is_relayed(d))
        .partition(|d| is_relayed(d));
    for d in open {
        let part = Part {
            kind: AttentionKind::Decision,
            target_id: d.id.clone(),
            title: d.title.clone(),
            created_at: d.created_at,
            target: Some(Target::DecisionId(d.id.clone())),
        };
        let legacy = json!({
            "kind": "decision", "id": d.id, "title": d.title,
            "priority": d.priority, "deadline_at": d.deadline_at, "relayed": false,
            "raised_by": d.raised_by_bot_id,
        });
        let row = b.push(part, decision_weight(d.priority), Some(legacy));
        row.priority = d.priority.as_str().to_string();
    }
    let rulings: Vec<Value> = relayed.iter().map(|d| relayed_ruling(d)).collect();
    if let Some(legacy) = relayed_row(&rulings) {
        let oldest = relayed
            .iter()
            .filter_map(|d| d.ruling.as_ref().map(|r| r.answered_at))
            .min()
            .unwrap_or_else(Utc::now);
        let part = Part {
            kind: AttentionKind::RelayedRulings,
            target_id: b.project_id.to_string(),
            title: format!("Confirm {} ruling(s) a bot recorded for you", rulings.len()),
            created_at: oldest,
            target: None,
        };
        let row = b.push(part, weight(AttentionKind::RelayedRulings), Some(legacy));
        row.relayed_count = u32::try_from(rulings.len()).unwrap_or(u32::MAX);
    }
    Ok(())
}

/// Open and settled decisions of the project, newest first.
fn project_decisions(app: &AppState, project_id: &str) -> anyhow::Result<Vec<Decision>> {
    app.db.list_decisions(&DecisionFilter {
        project_id: Some(project_id),
        states: &[DecisionState::Open, DecisionState::Settled],
        tag: None,
        bot_id: None,
        query: None,
        before: None,
        limit: 200,
    })
}

fn p0_items(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    let db = &app.db;
    let columns = db.board_columns(b.project_id)?;
    let closed = |key: &str| {
        columns.iter().any(|c| {
            c.key == key && matches!(c.category, ColumnCategory::Done | ColumnCategory::Cancelled)
        })
    };
    for card in db
        .board_cards(b.project_id)?
        .iter()
        .filter(|c| c.priority == Priority::P0 && !closed(&c.column_key))
    {
        let created_at = db
            .board_read(|t| t.item(&card.id))?
            .map_or_else(Utc::now, |item| item.created_at);
        let part = Part {
            kind: AttentionKind::P0Item,
            target_id: card.id.clone(),
            title: format!("{} {}", card.id, card.title),
            created_at,
            target: Some(Target::ItemId(card.id.clone())),
        };
        let legacy = json!({
            "kind": "p0", "id": card.id, "title": card.title,
            "column_key": card.column_key, "assignee": card.assignee,
        });
        b.push(part, weight(AttentionKind::P0Item), Some(legacy));
    }
    Ok(())
}

/// Commands proposed for the owner to run on this computer (H-117).
fn owner_actions(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    let now = Utc::now();
    let here: Vec<_> = app
        .db
        .list_owner_actions(Some(b.project_id), 200)?
        .into_iter()
        .filter(|a| a.state == ActionState::Proposed && a.expires_at > now)
        .filter(|a| a.proposal.target_machine == b.me)
        .collect();
    for action in here {
        let reason = action.proposal.reason.trim();
        let what = if reason.is_empty() {
            action.proposal.content.as_str()
        } else {
            reason
        };
        let part = Part {
            kind: AttentionKind::OwnerAction,
            target_id: action.id.clone(),
            title: format!("Run: {what}"),
            created_at: action.created_at,
            target: Some(Target::ActionId(action.id.clone())),
        };
        b.push(part, weight(AttentionKind::OwnerAction), None);
    }
    Ok(())
}

/// Permission prompts of the project's bots here, and its bots waiting for
/// the owner. Stand-ins run elsewhere: their computer counts them.
fn prompts_and_waiting(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    let bots: HashMap<String, bus::Bot> = app
        .db
        .list_bots(Some(b.project_id))?
        .into_iter()
        .filter(|bot| !bot.is_linked())
        .map(|bot| (bot.id.clone(), bot))
        .collect();
    for prompt in app.approvals.list(None) {
        let Some(bot) = bots.get(&prompt.bot_id) else {
            continue;
        };
        let part = Part {
            kind: AttentionKind::PermissionPrompt,
            target_id: prompt.id.clone(),
            title: format!("{} asks: {}", bot.name, prompt.summary),
            created_at: prompt.created_at,
            target: Some(Target::RequestId(prompt.id.clone())),
        };
        b.push(part, weight(AttentionKind::PermissionPrompt), None);
    }
    let mut waiting: Vec<&bus::Bot> = bots
        .values()
        .filter(|bot| app.supervisor.state(&bot.id).0 == BotState::WaitingForUser)
        .collect();
    waiting.sort_by(|a, b| a.name.cmp(&b.name));
    for bot in waiting {
        let reason = app.supervisor.state(&bot.id).1;
        let title = if reason.is_empty() {
            format!("{} is waiting for you", bot.name)
        } else {
            format!("{} is waiting for you: {reason}", bot.name)
        };
        let part = Part {
            kind: AttentionKind::BotWaiting,
            target_id: bot.id.clone(),
            title,
            created_at: app.overview.waiting_since(&bot.id),
            target: Some(Target::Bot(BotRef {
                daemon_id: b.me.clone(),
                bot_id: bot.id.clone(),
                name: bot.name.clone(),
            })),
        };
        b.push(part, weight(AttentionKind::BotWaiting), None);
    }
    Ok(())
}

/// Questions the project's bots here asked the owner (D6): in a bot's
/// thread, or on a card.
fn owner_questions(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    for q in crate::owner_threads::open_questions(app, Some(b.project_id), None)? {
        let target = match &q.item_id {
            Some(item_id) => Target::ItemId(item_id.clone()),
            None => {
                let Some(bot) = app.db.get_bot(&q.bot_id)? else {
                    continue;
                };
                Target::Bot(crate::owner_threads::bot_ref(app, &bot))
            }
        };
        let part = Part {
            kind: AttentionKind::OwnerQuestion,
            target_id: q.id.clone(),
            title: q.title.clone(),
            created_at: q.created_at,
            target: Some(target),
        };
        b.push(part, weight(AttentionKind::OwnerQuestion), None);
    }
    Ok(())
}
