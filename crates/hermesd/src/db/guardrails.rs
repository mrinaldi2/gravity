//! Guardrails part 2 (H-135): the card a routine's runs are for (G5),
//! which peer holds a linked project's board (ARCH-R59 a), and which tasks
//! count as on the board (ARCH-R61, H-158).

use bus::Message;
use rusqlite::{params, OptionalExtension};

use super::Db;

impl Db {
    /// Names the card a routine's runs are for; `None` clears it.
    pub fn set_routine_card(&self, routine_id: &str, card_id: Option<&str>) -> anyhow::Result<()> {
        let conn = self.lock();
        match card_id {
            Some(card) => conn.execute(
                "INSERT OR REPLACE INTO routine_card(routine_id, card_id) VALUES (?1, ?2)",
                params![routine_id, card],
            )?,
            None => conn.execute(
                "DELETE FROM routine_card WHERE routine_id = ?1",
                params![routine_id],
            )?,
        };
        Ok(())
    }

    /// The card a routine's runs are for, if it names one.
    pub fn routine_card(&self, routine_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT card_id FROM routine_card WHERE routine_id = ?1",
                params![routine_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Remembers that `peer_id` holds the project's board, across restarts.
    pub fn set_board_home(&self, project_id: &str, peer_id: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT OR REPLACE INTO board_home(project_id, peer_id) VALUES (?1, ?2)",
            params![project_id, peer_id],
        )?;
        Ok(())
    }

    /// Forgets the board's home, when `peer_id` says it no longer holds it
    /// or the link is gone.
    pub fn forget_board_home(&self, project_id: &str, peer_id: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "DELETE FROM board_home WHERE project_id = ?1 AND peer_id = ?2",
            params![project_id, peer_id],
        )?;
        Ok(())
    }

    /// Whether a task is on the board: it names a card, or it is a release's
    /// deploy or rollback, which the release itself accounts for (ARCH-R61
    /// M2), here or on the computer that forwarded it (H-158).
    pub fn task_on_board(&self, task_id: &str) -> anyhow::Result<bool> {
        if self.task_card(task_id)?.is_some() {
            return Ok(true);
        }
        Ok(self.task_release(task_id)?.is_some())
    }

    /// Names the release a deploy or rollback task is for: as it opens on
    /// the release's home, or as a peer's frame says (H-158).
    pub fn set_task_release(&self, task_id: &str, release_id: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT OR REPLACE INTO task_release(task_id, release_id) VALUES (?1, ?2)",
            params![task_id, release_id],
        )?;
        Ok(())
    }

    /// The release a task deploys or rolls back, if it is one.
    pub fn task_release(&self, task_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT coalesce(
                     (SELECT release_id FROM task_release WHERE task_id = ?1),
                     (SELECT release_id FROM release_deployment WHERE task_id = ?1 LIMIT 1))",
                params![task_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Where a deploy or rollback task installs (H-288).
    pub fn set_task_target(&self, task_id: &str, target: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT OR REPLACE INTO task_target(task_id, target) VALUES (?1, ?2)",
            params![task_id, target],
        )?;
        Ok(())
    }

    pub fn task_target(&self, task_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT target FROM task_target WHERE task_id = ?1",
                params![task_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// An install under way here (H-166): a bot on this computer holds an
    /// open deploy or rollback task that installs on this computer, its
    /// install not confirmed yet (a confirmed one is done whether or not its
    /// tester has closed the task). The
    /// release and the bot's name. An install on the iPhone (`ios`/`iphone`)
    /// or on a linked computer isn't one here, and a task with no known
    /// target (DevOps asked to roll back, or a peer older than H-288) installs
    /// nothing by itself (H-288).
    pub fn install_task_here(&self) -> anyhow::Result<Option<(String, String)>> {
        let rows: Vec<(String, String, Option<String>)> = {
            let conn = self.lock();
            let mut stmt = conn.prepare(
                "SELECT coalesce(tr.release_id, rd.release_id), b.name,
                        coalesce(tt.target, rd.machine)
                 FROM task t JOIN bot b ON b.id = t.to_bot_id
                 LEFT JOIN task_release tr ON tr.task_id = t.id
                 LEFT JOIN release_deployment rd ON rd.task_id = t.id
                 LEFT JOIN task_target tt ON tt.task_id = t.id
                 WHERE t.state = 'open' AND b.peer_id IS NULL
                   AND coalesce(tr.release_id, rd.release_id) IS NOT NULL
                   AND (rd.task_id IS NULL OR rd.result IS NULL)
                 ORDER BY t.created_at",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let elsewhere = self.board_read(|t| t.peer_names())?;
        Ok(rows.into_iter().find_map(|(release, bot, target)| {
            let target = target?;
            let here = !crate::board::release::machines::is_ios_target(&target)
                && !elsewhere.iter().any(|p| p.eq_ignore_ascii_case(&target));
            here.then_some((release, bot))
        }))
    }

    /// The message with this number, as an envelope names it.
    pub fn message_by_num(&self, num: i64) -> anyhow::Result<Option<Message>> {
        let id: Option<String> = self
            .lock()
            .query_row("SELECT id FROM message WHERE num = ?1", params![num], |r| {
                r.get(0)
            })
            .optional()?;
        match id {
            Some(id) => self.get_message(&id),
            None => Ok(None),
        }
    }

    /// The peer last known to hold the project's board.
    pub fn board_home(&self, project_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT peer_id FROM board_home WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use bus::{MessageKind, TaskState};

    use crate::db::tests::{setup, user_sender};
    use crate::db::Db;

    #[test]
    fn routine_cards_and_board_homes_round_trip() {
        let db = Db::open_in_memory().expect("open");
        db.set_routine_card("r1", Some("H-1")).expect("set");
        assert_eq!(db.routine_card("r1").expect("get").as_deref(), Some("H-1"));
        db.set_routine_card("r1", None).expect("clear");
        assert_eq!(db.routine_card("r1").expect("get"), None);

        db.set_board_home("p1", "peer-a").expect("set");
        assert_eq!(db.board_home("p1").expect("get").as_deref(), Some("peer-a"));
        // Another peer's word doesn't forget it.
        db.forget_board_home("p1", "peer-b").expect("forget");
        assert!(db.board_home("p1").expect("get").is_some());
        db.forget_board_home("p1", "peer-a").expect("forget");
        assert_eq!(db.board_home("p1").expect("get"), None);
    }

    /// ARCH-R63 S2: retention takes a pruned task's release with it, as it
    /// does its card; a kept task keeps both.
    #[test]
    fn retention_prunes_a_tasks_release_with_the_task() {
        let (db, bot) = setup();
        let conv = db.dm_conversation(&bot.id).unwrap().unwrap();
        let task = |body: &str| {
            let msg = db
                .insert_message(
                    &conv.id,
                    &user_sender(),
                    MessageKind::Task,
                    body,
                    None,
                    None,
                )
                .unwrap();
            let task = db.create_task(&msg.id, None, &bot.id, None, 1, "").unwrap();
            db.set_task_card(&task.id, "H-1").unwrap();
            db.set_task_release(&task.id, "r1").unwrap();
            task.id
        };
        let (old, kept) = (task("old"), task("kept"));
        db.try_close_task(&old, TaskState::Done).unwrap();
        db.lock()
            .execute(
                "UPDATE task SET created_at = '2000-01-01T00:00:00Z' WHERE id = ?1",
                rusqlite::params![old],
            )
            .unwrap();
        db.prune(1, 1, 1, 1).unwrap();
        assert!(db.get_task(&old).unwrap().is_none(), "the task went");
        assert_eq!(db.task_card(&old).unwrap(), None);
        assert_eq!(db.task_release(&old).unwrap(), None, "and its release");
        assert_eq!(db.task_release(&kept).unwrap().as_deref(), Some("r1"));
        assert_eq!(db.task_card(&kept).unwrap().as_deref(), Some("H-1"));
    }

    /// H-181: a forwarded deploy task and its release commit together. Once
    /// the task is visible its release is too, and if the release can't be
    /// written the task isn't either.
    #[test]
    fn a_task_and_its_release_commit_together() {
        let (db, bot) = setup();
        let conv = db.dm_conversation(&bot.id).unwrap().unwrap();
        let msg = |body: &str| {
            db.insert_message(
                &conv.id,
                &user_sender(),
                MessageKind::Task,
                body,
                None,
                None,
            )
            .unwrap()
            .id
        };
        let task = db
            .create_task_with_release(&msg("deploy"), None, &bot.id, None, 1, "", Some("r1"))
            .unwrap();
        assert_eq!(db.task_release(&task.id).unwrap().as_deref(), Some("r1"));
        let plain = db
            .create_task_with_release(&msg("plain"), None, &bot.id, None, 1, "", None)
            .unwrap();
        assert_eq!(db.task_release(&plain.id).unwrap(), None);

        // The release row fails to write: the task rolls back with it.
        db.lock()
            .execute_batch("ALTER TABLE task_release RENAME TO task_release_gone")
            .unwrap();
        let before = db.open_tasks_for(&bot.id).unwrap().len();
        assert!(db
            .create_task_with_release(&msg("lost"), None, &bot.id, None, 1, "", Some("r2"))
            .is_err());
        assert_eq!(
            db.open_tasks_for(&bot.id).unwrap().len(),
            before,
            "no task alone"
        );
    }
}
