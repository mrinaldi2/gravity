//! What a release waits on from the owner (H-247, UX-048 §1): its own
//! ruling, and the Run cards, decisions, permission prompts and questions on
//! its work card or its items. Computed when a release is shown; the shape
//! is frozen in artifacts/06d0acd4-H-247-owner-blockers-shape.md.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::machines;
use super::model::{Release, ReleaseStatus};
use crate::app::AppState;
use crate::db::OnCard;
use crate::decisions::invalid;

/// One thing only the owner can clear that holds the release up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub kind: &'static str,
    pub id: String,
    /// Raw text: the client words it (UX-048 §2).
    pub title: String,
    pub item_id: Option<String>,
    pub bot: Option<String>,
    pub computer: Option<String>,
    pub created_at: String,
}

impl Blocker {
    pub fn to_json(&self) -> Value {
        json!({
            "kind": self.kind, "id": self.id, "title": self.title, "item_id": self.item_id,
            "bot": self.bot, "computer": self.computer, "created_at": self.created_at,
        })
    }

    /// The needs-you order (UX-024): a ruling, then Run cards, decisions and
    /// permission prompts, then questions.
    fn rank(&self) -> u8 {
        match self.kind {
            "ruling" => 0,
            "question" => 2,
            _ => 1,
        }
    }
}

/// The first line of `text`, trimmed.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}

/// A bot id, from `bot:<id>` or a bare one.
fn bot_id(by: &str) -> String {
    by.strip_prefix("bot:").unwrap_or(by).to_string()
}

/// `item_id` is a card on `project`'s board.
pub fn own_card(app: &AppState, project: &str, item_id: &str) -> anyhow::Result<()> {
    let on = app.db.board_read(|t| t.item_project(item_id))?;
    if on.as_deref() != Some(project) {
        return Err(invalid(format!(
            "no card {item_id} on this project's board"
        )));
    }
    Ok(())
}

/// Sets a release's work card: a card on its own board, while the release is
/// open. Any stage, unlike the package's text.
pub fn set_work_item(
    app: &AppState,
    caller: &super::Caller<'_>,
    release_id: &str,
    item_id: &str,
) -> anyhow::Result<()> {
    let project = caller.bot.project_id.as_str();
    app.db.board_tx(|t| {
        let release = super::load(t, project, release_id)?;
        if release.status.is_closed() {
            return Err(invalid(format!("release {release_id} is closed")));
        }
        if t.item_project(item_id)?.as_deref() != Some(project) {
            return Err(invalid(format!(
                "no card {item_id} on this project's board"
            )));
        }
        t.set_release_work_item(release_id, item_id, &format!("bot:{}", caller.bot.id))
    })
}

/// A release as clients and bots see it: its JSON with `owner_blockers`.
/// A blocker that can't be read leaves the list empty, never the release.
pub fn payload(app: &AppState, release: &Release) -> Value {
    let mut v = release.to_json();
    let blockers = owner_blockers(app, release).unwrap_or_else(|e| {
        tracing::warn!(release = %release.id, error = %e, "couldn't read what a release waits on");
        Vec::new()
    });
    v["owner_blockers"] = blockers.iter().map(Blocker::to_json).collect();
    v
}

/// Everything holding `release` up for the owner, in the needs-you order.
/// A closed release waits on nothing.
pub fn owner_blockers(app: &AppState, release: &Release) -> anyhow::Result<Vec<Blocker>> {
    if release.status.is_closed() {
        return Ok(Vec::new());
    }
    let mut cards: BTreeSet<String> = release.items.iter().map(|i| i.item_id.clone()).collect();
    cards.extend(release.work_item_id.clone());
    let cards_json = serde_json::to_string(&cards)?;
    let project = release.project_id.as_str();
    let me = app.db.daemon_id()?;
    let (runs, decisions, here) = app.db.board_read(|t| {
        Ok((
            t.proposed_actions_on(project, &cards_json)?,
            t.open_decisions_on(project, &cards_json)?,
            machines::this_computer(t)?,
        ))
    })?;
    let computer = |target: &str| {
        if target == me {
            here.clone()
        } else {
            crate::peer::owner_actions::target_name(app, target)
        }
    };
    let mut out = Vec::new();
    if release.status == ReleaseStatus::AwaitingOwner {
        out.push(Blocker {
            kind: "ruling",
            id: release
                .decision_id
                .clone()
                .unwrap_or_else(|| release.id.clone()),
            title: release
                .display_version
                .clone()
                .unwrap_or_else(|| release.name.clone()),
            item_id: release.work_item_id.clone(),
            bot: None,
            computer: None,
            created_at: release.updated_at.to_rfc3339(),
        });
    }
    let card = |kind: &'static str, c: OnCard, computer: Option<String>| Blocker {
        kind,
        id: c.id,
        title: first_line(&c.title),
        item_id: Some(c.item_id),
        bot: Some(bot_id(&c.by)),
        computer,
        created_at: c.created_at,
    };
    for run in runs {
        let target = run.target.as_deref().map(computer);
        out.push(card("run", run, target));
    }
    for decision in decisions {
        out.push(card("decision", decision, None));
    }
    out.extend(permissions(app, &cards, &here)?);
    out.extend(questions(app, project, &cards)?);
    out.sort_by(|a, b| (a.rank(), &a.created_at).cmp(&(b.rank(), &b.created_at)));
    Ok(out)
}

/// Permission prompts from bots whose open task is on one of `cards`.
fn permissions(
    app: &AppState,
    cards: &BTreeSet<String>,
    here: &str,
) -> anyhow::Result<Vec<Blocker>> {
    let mut out = Vec::new();
    for request in app.approvals.list(None) {
        let on = app.db.board_read(|t| t.open_task_cards(&request.bot_id))?;
        if let Some(item) = on.into_iter().find(|c| cards.contains(c)) {
            out.push(Blocker {
                kind: "permission",
                id: request.id.clone(),
                title: request.tool.clone(),
                item_id: Some(item),
                bot: Some(request.bot_id.clone()),
                computer: Some(here.to_string()),
                created_at: request.created_at.to_rfc3339(),
            });
        }
    }
    Ok(out)
}

/// Open `asks_owner` questions on one of `cards`, by the comment that asks.
fn questions(
    app: &AppState,
    project: &str,
    cards: &BTreeSet<String>,
) -> anyhow::Result<Vec<Blocker>> {
    let mut out = Vec::new();
    for q in crate::owner_threads::open_questions(app, Some(project), None)? {
        let Some(item) = q.item_id.clone().filter(|i| cards.contains(i)) else {
            continue;
        };
        // The question as asked: the comment's own first line, not the
        // thread's "<bot> asks on <card>: …" title.
        let comment = app.db.question_comment(&q.id)?;
        let asked = match &comment {
            Some(id) => app
                .db
                .board_read(|t| t.item_comments(&item))?
                .into_iter()
                .find(|c| &c.id == id)
                .map(|c| first_line(&c.body)),
            None => None,
        };
        out.push(Blocker {
            kind: "question",
            id: comment.unwrap_or(q.id),
            title: asked.unwrap_or_else(|| first_line(&q.title)),
            item_id: Some(item),
            bot: Some(q.bot_id),
            computer: None,
            created_at: q.created_at.to_rfc3339(),
        });
    }
    Ok(out)
}
