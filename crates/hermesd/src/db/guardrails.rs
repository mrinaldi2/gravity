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
}
