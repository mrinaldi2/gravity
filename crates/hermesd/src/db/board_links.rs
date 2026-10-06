//! Board storage, part 6: the bus's links to items (H-020 §1.6). A task or
//! decision raised for an item names it and appears on the item as a link;
//! a task's end is written to the item's history in the same transaction
//! that closes it.

use bus::{now, TaskState};
use rusqlite::{params, Connection, OptionalExtension};

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

impl Db {
    /// Name the card a task is for without linking it here: the card lives
    /// on another machine's board (H-125).
    pub fn set_task_card(&self, task_id: &str, card_id: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT OR REPLACE INTO task_card(task_id, card_id) VALUES (?1, ?2)",
            params![task_id, card_id],
        )?;
        Ok(())
    }

    /// The card a task is for, linked here or not. `None` for a task opened
    /// before cards were required, or forwarded by an older peer.
    pub fn task_card(&self, task_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT coalesce(
                     (SELECT card_id FROM task_card WHERE task_id = ?1), item_id)
                 FROM task WHERE id = ?1",
                params![task_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// The card a spawn's task will be for, kept until that task exists.
    pub fn set_worker_card(&self, worker_id: &str, card_id: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT OR REPLACE INTO worker_card(worker_id, card_id) VALUES (?1, ?2)",
            params![worker_id, card_id],
        )?;
        Ok(())
    }

    pub fn worker_card(&self, worker_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT card_id FROM worker_card WHERE worker_id = ?1",
                params![worker_id],
                |r| r.get(0),
            )
            .optional()?)
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
