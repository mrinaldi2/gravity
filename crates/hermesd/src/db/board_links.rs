//! Board storage, part 6: the bus's links to items (H-020 §1.6). A task or
//! decision raised for an item names it and appears on the item as a link;
//! a task's end is written to the item's history in the same transaction
//! that closes it.

use bus::{now, TaskState};
use rusqlite::{params, Connection};

use crate::board::model::{ItemEventKind, LinkKind};

use super::board::to_text;
use super::{ts, Actor, Db};

impl Db {
    /// Mark a task as delegated for an item and link it there.
    pub fn link_task_item(
        &self,
        task_id: &str,
        item_id: &str,
        actor: &Actor<'_>,
    ) -> anyhow::Result<()> {
        self.board_tx(|t| {
            t.conn.execute(
                "UPDATE task SET item_id = ?2 WHERE id = ?1",
                params![task_id, item_id],
            )?;
            t.add_item_link(item_id, LinkKind::Task, task_id, None, actor)?;
            Ok(())
        })
    }

    /// Mark a decision as raised for an item and link it there.
    pub fn link_decision_item(
        &self,
        decision_id: &str,
        item_id: &str,
        actor: &Actor<'_>,
    ) -> anyhow::Result<()> {
        self.board_tx(|t| {
            t.conn.execute(
                "UPDATE decision SET item_id = ?2 WHERE id = ?1",
                params![decision_id, item_id],
            )?;
            t.add_item_link(item_id, LinkKind::Decision, decision_id, None, actor)?;
            Ok(())
        })
    }

    /// The item a task was delegated for, if any.
    pub fn task_item(&self, task_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self.lock().query_row(
            "SELECT item_id FROM task WHERE id = ?1",
            params![task_id],
            |r| r.get(0),
        )?)
    }
}

/// Write a closed task's end to its item's history, when it has one. The
/// actor is who closed it: the assignee (done), the requester (cancelled) or
/// the daemon (expired).
pub(super) fn task_closed(
    conn: &Connection,
    task_id: &str,
    state: TaskState,
) -> rusqlite::Result<()> {
    let kind = match state {
        TaskState::Done => ItemEventKind::TaskDone,
        TaskState::Cancelled => ItemEventKind::TaskCancelled,
        TaskState::Expired => ItemEventKind::TaskExpired,
        TaskState::Open => return Ok(()),
    };
    conn.execute(
        r#"INSERT INTO item_event(item_id, project_id, at, actor, kind, "to", field)
           SELECT t.item_id, i.project_id, ?2,
                  CASE ?3 WHEN 'task_done' THEN 'bot:' || t.to_bot_id
                          WHEN 'task_cancelled' THEN coalesce('bot:' || t.from_bot_id, 'user')
                          ELSE 'daemon' END,
                  ?3, t.id, 'task'
           FROM task t JOIN item i ON i.id = t.item_id WHERE t.id = ?1"#,
        params![task_id, ts(now()), to_text(&kind)],
    )?;
    Ok(())
}
