//! The move engine (H-020 §1.2). `item_move` runs the guards and commits;
//! `item_move_check` runs them for every column and writes nothing. WS, MCP
//! and the peer link all come through here, so each rule lives only in
//! `guards`.

use crate::actor::Actor;
use crate::db::{Db, Write};

use super::guards::{self, unmet, Move, Rule, Who};
use super::model::{BoardColumn, Item, Unmet};

pub struct MoveRequest<'a> {
    pub id: &'a str,
    pub to: &'a str,
    pub expected_version: u64,
    pub reason: Option<&'a str>,
    pub override_reason: Option<&'a str>,
}

#[derive(Debug)]
pub enum Moved {
    Done(Box<Item>),
    /// The guards that are not met; nothing was written.
    Refused(Vec<Unmet>),
    /// The item changed since the caller read it; this is how it is now.
    Conflict(Box<Item>),
}

/// The item, its project and the actor as the guards see it.
fn load(db: &Db, id: &str, actor: &Actor<'_>) -> anyhow::Result<(Item, String, Who)> {
    let item = db
        .get_item(id)?
        .ok_or_else(|| anyhow::anyhow!("item {id} not found"))?;
    let project_id = db
        .item_project(id)?
        .ok_or_else(|| anyhow::anyhow!("item {id} not found"))?;
    let who = match actor {
        Actor::User | Actor::Device(_) => Who::Owner,
        Actor::Bot {
            id: bot,
            project_id: own,
        } => {
            anyhow::ensure!(*own == project_id, "item {id} belongs to another project");
            let roles = db
                .project_roles(&project_id)?
                .into_iter()
                .filter(|r| r.bot_id == *bot)
                .map(|r| r.role)
                .collect();
            Who::Bot {
                id: bot.to_string(),
                roles,
            }
        }
    };
    Ok((item, project_id, who))
}

/// Move an item if every guard holds. A stale `expected_version` is a
/// conflict before any guard runs, so refusals always describe the current card.
pub fn item_move(db: &Db, req: &MoveRequest<'_>, actor: &Actor<'_>) -> anyhow::Result<Moved> {
    let (item, project_id, who) = load(db, req.id, actor)?;
    if item.version != req.expected_version {
        return Ok(Moved::Conflict(Box::new(item)));
    }
    let columns = db.board_columns(&project_id)?;
    let Some(to) = columns.iter().find(|c| c.key == req.to) else {
        let text = format!("This board has no column {}.", req.to);
        return Ok(Moved::Refused(vec![unmet("move.column", text, None)]));
    };
    let ctx = db.move_context(&project_id, &item)?;
    let mv = Move {
        to,
        reason: req.reason,
        override_reason: req.override_reason,
    };
    let refused = guards::evaluate(&item, &mv, &who, &ctx);
    if !refused.is_empty() {
        return Ok(Moved::Refused(refused));
    }
    let over = guards::over_limit(&item, to, &ctx).is_some() && who != Who::Daemon;
    let note = note(&item, &mv, over);
    Ok(
        match db.move_item(
            req.id,
            req.expected_version,
            req.to,
            note.as_deref(),
            over,
            actor,
        )? {
            Write::Done(item) => Moved::Done(Box::new(item)),
            Write::Conflict(current) => Moved::Conflict(current),
        },
    )
}

/// What the move's history event says: the reason, the review verdict and
/// any WIP override.
fn note(item: &Item, mv: &Move<'_>, over: bool) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(reason) = mv.reason.map(str::trim).filter(|r| !r.is_empty()) {
        parts.push(reason.to_string());
    }
    if guards::rule(item.category, mv.to.category) == Rule::Approve {
        parts.push("verdict: approve".to_string());
    }
    if over {
        let reason = mv.override_reason.unwrap_or_default().trim();
        parts.push(format!("WIP override: {reason}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What stands between the item and each other column, in board order, for
/// the actor asking. No reason or override is assumed, so a column that needs
/// one lists it.
pub fn item_move_check(
    db: &Db,
    id: &str,
    actor: &Actor<'_>,
) -> anyhow::Result<Vec<(BoardColumn, Vec<Unmet>)>> {
    let (item, project_id, who) = load(db, id, actor)?;
    let ctx = db.move_context(&project_id, &item)?;
    Ok(db
        .board_columns(&project_id)?
        .into_iter()
        .filter(|c| c.key != item.column_key)
        .map(|to| {
            let mv = Move {
                to: &to,
                reason: None,
                override_reason: None,
            };
            let refused = guards::evaluate(&item, &mv, &who, &ctx);
            (to, refused)
        })
        .collect())
}

#[cfg(test)]
mod tests;
