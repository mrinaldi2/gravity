//! The move engine (H-020 §1.2). `item_move` runs the guards and commits;
//! `item_move_check` runs them for every column and writes nothing. WS, MCP
//! and the peer link all come through here, so each rule lives only in
//! `guards`.

use crate::actor::Actor;
use crate::db::{BoardTx, Db, MoveTo, Write};

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

/// The item, its project and the actor as the guards see it, read inside
/// the caller's transaction. A bot of another project is refused here.
pub(crate) fn load_in(
    t: &BoardTx<'_>,
    id: &str,
    actor: &Actor<'_>,
) -> anyhow::Result<(Item, String, Who)> {
    let missing = || anyhow::anyhow!("item {id} not found");
    let item = t.item(id)?.ok_or_else(missing)?;
    let project_id = t.item_project(id)?.ok_or_else(missing)?;
    let who = match actor {
        Actor::User | Actor::Device(_) => Who::Owner,
        Actor::Bot {
            id: bot,
            project_id: own,
        } => {
            anyhow::ensure!(*own == project_id, "item {id} belongs to another project");
            let roles = t
                .roles(&project_id)?
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

/// Move an item if every guard holds, all in one transaction (ARCH-R4 §3).
/// A stale `expected_version` is a conflict before any guard runs, so
/// refusals always describe the current card.
pub fn item_move(db: &Db, req: &MoveRequest<'_>, actor: &Actor<'_>) -> anyhow::Result<Moved> {
    db.board_tx(|t| {
        let (item, project_id, who) = load_in(t, req.id, actor)?;
        if item.version != req.expected_version {
            return Ok(Moved::Conflict(Box::new(item)));
        }
        let columns = t.columns(&project_id)?;
        let Some(to) = columns.iter().find(|c| c.key == req.to) else {
            let text = format!("This board has no column {}.", req.to);
            return Ok(Moved::Refused(vec![unmet("move.column", text, None)]));
        };
        let ctx = t.move_context(&project_id, &item)?;
        let mv = Move {
            to,
            reason: req.reason,
            override_reason: req.override_reason,
        };
        let refused = guards::evaluate(&item, &mv, &who, &ctx);
        if !refused.is_empty() {
            return Ok(Moved::Refused(refused));
        }
        let rule = guards::rule(item.category, to.category);
        let over = guards::over_limit(&item, to, &ctx).is_some() && who != Who::Daemon;
        let note = note(rule, &mv, over);
        let first = guards::is_return(rule);
        Ok(
            match t.move_item(
                req.id,
                req.expected_version,
                &MoveTo {
                    column: req.to,
                    note: note.as_deref(),
                    wip_override: over,
                    first,
                },
                actor,
            )? {
                Write::Done(item) => Moved::Done(Box::new(item)),
                Write::Conflict(current) => Moved::Conflict(current),
            },
        )
    })
}

/// What the move's history event says: the reason, the review verdict and
/// any WIP override, automatic for returned work (H-017 rev 2.1 §1.3).
fn note(rule: Rule, mv: &Move<'_>, over: bool) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(reason) = mv.reason.map(str::trim).filter(|r| !r.is_empty()) {
        parts.push(reason.to_string());
    }
    if rule == Rule::Approve {
        parts.push("verdict: approve".to_string());
    }
    if over && guards::is_return(rule) {
        parts.push(guards::RETURNED_OVER_WIP.to_string());
    } else if over {
        let reason = mv.override_reason.unwrap_or_default().trim();
        parts.push(format!("WIP override: {reason}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What stands between the item and each other column, in board order, for
/// the actor asking, read from one snapshot. No reason or override is
/// assumed, so a column that needs one lists it.
pub fn item_move_check(
    db: &Db,
    id: &str,
    actor: &Actor<'_>,
) -> anyhow::Result<Vec<(BoardColumn, Vec<Unmet>)>> {
    db.board_read(|t| {
        let (item, project_id, who) = load_in(t, id, actor)?;
        let ctx = t.move_context(&project_id, &item)?;
        Ok(t.columns(&project_id)?
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
    })
}

#[cfg(test)]
mod tests;
