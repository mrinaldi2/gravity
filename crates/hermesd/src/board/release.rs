//! Release packages and the deploy gate (H-020 §2, §6; H-017 §1.4).
//!
//! DevOps assembles a package (`assemble`), the owner rules on it from the
//! dashboard or phone (`rule`), and DevOps rolls it out through each
//! machine's tester (`deploy`). The gate is that only `rule`, called with the
//! owner's own credentials on the board's home, can move items to deploying,
//! and `deploy` re-checks the ruling every time it is used.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::feed::{card_after_commit, Change, ChangeKind, FeedWriter};
use crate::board::guards::{self, Move, Who};
use crate::board::model::{ColumnCategory, Role};
use crate::db::{BoardTx, MoveTo, Write};
use crate::decisions::{conflict, forbidden, not_found};

use model::Release;

pub mod assemble;
pub mod deploy;
pub mod lifecycle;
pub mod model;
pub mod package;
pub mod publish;
pub mod rule;
pub mod serve;
#[cfg(test)]
mod tests;

/// A bot acting on releases, with its board roles.
pub struct Caller<'a> {
    pub bot: &'a bus::Bot,
    pub roles: Vec<Role>,
}

impl Caller<'_> {
    pub fn load<'a>(app: &Arc<AppState>, bot: &'a bus::Bot) -> anyhow::Result<Caller<'a>> {
        let roles = app
            .db
            .project_roles(&bot.project_id)?
            .into_iter()
            .filter(|r| r.bot_id == bot.id)
            .map(|r| r.role)
            .collect();
        Ok(Caller { bot, roles })
    }

    pub fn actor(&self) -> Actor<'_> {
        Actor::Bot {
            id: &self.bot.id,
            project_id: &self.bot.project_id,
        }
    }

    pub fn has(&self, role: Role) -> bool {
        self.roles.contains(&role)
    }

    /// Refused unless the bot holds `role`.
    pub fn require(&self, role: Role, what: &str) -> anyhow::Result<()> {
        if self.has(role) {
            Ok(())
        } else {
            Err(forbidden(format!(
                "only a bot with the {} role can {what}; ask the lead to assign it",
                role.as_str()
            )))
        }
    }
}

/// A package of the caller's project; another project's packages don't
/// exist for it.
pub fn load(t: &BoardTx<'_>, project_id: &str, id: &str) -> anyhow::Result<Release> {
    t.release(id)?
        .filter(|r| r.project_id == project_id)
        .ok_or_else(|| not_found(format!("no release {id} in this project")))
}

/// What a package is, for the gate: its items, its builds and the tests run
/// against them. Taken at submit and compared at the ruling and at every
/// deploy, so a package can't change under an approval.
pub fn frozen_hash(release: &Release) -> String {
    let mut h = Sha256::new();
    let mut items: Vec<&str> = release.items.iter().map(|i| i.item_id.as_str()).collect();
    items.sort_unstable();
    for item in items {
        h.update(format!("item\0{item}\n"));
    }
    for b in &release.builds {
        h.update(format!(
            "build\0{}\0{}\0{}\n",
            b.platform, b.version, b.sha256
        ));
    }
    for t in &release.tests {
        h.update(format!(
            "test\0{}\0{}\0{}\n",
            t.machine, t.build_sha256, t.result
        ));
    }
    hex::encode(h.finalize())
}

/// Refused with "resubmit" when the package changed since it was frozen.
pub fn check_frozen(release: &Release) -> anyhow::Result<()> {
    match &release.frozen_hash {
        Some(hash) if *hash == frozen_hash(release) => Ok(()),
        Some(_) => Err(conflict(format!(
            "release {} changed after it was submitted; DevOps must resubmit it",
            release.name
        ))),
        None => Err(conflict(format!(
            "release {} has not been submitted",
            release.name
        ))),
    }
}

/// An item move the release makes on its own authority (H-020 §2.3): the
/// guards' daemon path, recorded under `actor` (the owner who ruled, or the
/// tester who confirmed). Returns the column it left, or `None` when it was
/// already there.
pub fn daemon_move(
    t: &BoardTx<'_>,
    project_id: &str,
    item_id: &str,
    to: ColumnCategory,
    note: &str,
    first: bool,
    actor: &Actor<'_>,
) -> anyhow::Result<Option<String>> {
    let item = t
        .item(item_id)?
        .ok_or_else(|| not_found(format!("no item {item_id}")))?;
    let columns = t.columns(project_id)?;
    let column = columns
        .iter()
        .find(|c| c.category == to)
        .ok_or_else(|| anyhow::anyhow!("this board has no {} column", to.as_str()))?;
    if column.key == item.column_key {
        return Ok(None);
    }
    let ctx = t.move_context(project_id, &item)?;
    let mv = Move {
        to: column,
        reason: Some(note),
        override_reason: None,
    };
    let unmet = guards::evaluate(&item, &mv, &Who::Daemon, &ctx);
    anyhow::ensure!(
        unmet.is_empty(),
        "the daemon can't move {item_id}: {unmet:?}"
    );
    // Returned work over a WIP limit is flagged as F1 asks; nothing else is.
    let over = first && guards::over_limit(&item, column, &ctx).is_some();
    let note = if over {
        format!("{note} · {}", guards::RETURNED_OVER_WIP)
    } else {
        note.to_string()
    };
    let to = MoveTo {
        column: &column.key,
        note: Some(&note),
        wip_override: over,
        first,
    };
    match t.move_item(item_id, item.version, &to, actor)? {
        Write::Done(_) => Ok(Some(item.column_key)),
        Write::Conflict(_) => Err(conflict(format!("{item_id} changed while moving it"))),
    }
}

/// Push each move the release made, once its transaction has committed.
pub fn publish_moves(
    app: &Arc<AppState>,
    feed: &mut FeedWriter<'_>,
    project_id: &str,
    moved: &[(String, String)],
) {
    for (item_id, from) in moved {
        feed.publish(Change {
            project_id,
            kind: ChangeKind::ItemMoved,
            item_id,
            card: card_after_commit(&app.db, item_id),
            from_column: Some(from.clone()),
        });
    }
}

/// The bot ids of a project's testers on `machine`.
pub fn testers_on(
    app: &Arc<AppState>,
    project_id: &str,
    machine: &str,
) -> anyhow::Result<Vec<String>> {
    Ok(app
        .db
        .project_roles(project_id)?
        .into_iter()
        .filter(|r| r.role == Role::Tester && r.machine.as_deref() == Some(machine))
        .map(|r| r.bot_id)
        .collect())
}
