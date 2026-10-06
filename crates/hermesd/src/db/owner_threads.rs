//! Owner threads (H-128 R2.2, D6) and project pins (D7). A bot's owner
//! thread is its DM conversation's messages from the owner and the bot's own
//! `message_owner` notes; other bots' messages in that conversation aren't
//! part of it.

use bus::{new_id, now, Message};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{parse_ts, ts, Db};

/// A message the owner wrote. The daemon's own notices are stored as user
/// messages too, under another name, and aren't the owner's.
const FROM_OWNER: &str = "(m.sender_kind = 'user' AND m.sender_name = 'user')";

/// A message of the thread: the owner's, or the bot's own to the owner.
const IN_THREAD: &str = "((m.sender_kind = 'user' AND m.sender_name = 'user')
     OR (m.sender_kind = 'bot' AND m.sender_bot_id = c.bot_id))";

/// A question a bot asked the owner.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnerQuestion {
    pub id: String,
    pub bot_id: String,
    pub project_id: String,
    /// In the thread: the message that asked.
    pub message_num: Option<i64>,
    /// On a card: the card.
    pub item_id: Option<String>,
    pub title: String,
    pub created_at: DateTime<Utc>,
}

/// Where a question was asked.
pub enum Asked<'a> {
    Thread(i64),
    Card(&'a str),
}

const QUESTION_COLS: &str =
    "q.id, q.bot_id, q.project_id, q.message_num, q.item_id, q.title, q.created_at";

fn question_from_row(r: &Row<'_>) -> rusqlite::Result<OwnerQuestion> {
    Ok(OwnerQuestion {
        id: r.get(0)?,
        bot_id: r.get(1)?,
        project_id: r.get(2)?,
        message_num: r.get(3)?,
        item_id: r.get(4)?,
        title: r.get(5)?,
        created_at: parse_ts(&r.get::<_, String>(6)?),
    })
}

impl Db {
    /// A page of the bot's thread, oldest first, and whether older exist.
    pub fn owner_thread_page(
        &self,
        bot_id: &str,
        before_num: Option<i64>,
        limit: u32,
    ) -> anyhow::Result<(Vec<Message>, bool)> {
        let sql = format!(
            "SELECT {} FROM message m JOIN conversation c ON c.id = m.conversation_id
              WHERE c.bot_id = ?1 AND m.num < ?2 AND {IN_THREAD}
              ORDER BY m.num DESC LIMIT ?3",
            Self::MSG_COLS
                .split(", ")
                .map(|col| format!("m.{col}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let conn = self.lock();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![bot_id, before_num.unwrap_or(i64::MAX), i64::from(limit) + 1],
            Self::message_from_row,
        )?;
        let mut messages: Vec<Message> = rows.collect::<Result<_, _>>()?;
        let has_more = messages.len() > limit as usize;
        messages.truncate(limit as usize);
        messages.reverse();
        Ok((messages, has_more))
    }

    /// The thread's newest message.
    pub fn owner_thread_last(&self, bot_id: &str) -> anyhow::Result<Option<Message>> {
        Ok(self.owner_thread_page(bot_id, None, 1)?.0.pop())
    }

    /// How far the owner has read the bot's thread; 0 when never.
    pub fn owner_last_read(&self, bot_id: &str) -> anyhow::Result<i64> {
        Ok(self
            .lock()
            .query_row(
                "SELECT last_read_num FROM owner_read WHERE bot_id = ?1",
                params![bot_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    /// The bot's messages to the owner after the last one read.
    pub fn owner_unread(&self, bot_id: &str) -> anyhow::Result<u32> {
        let read = self.owner_last_read(bot_id)?;
        Ok(self.lock().query_row(
            "SELECT count(*) FROM message m JOIN conversation c ON c.id = m.conversation_id
              WHERE c.bot_id = ?1 AND m.num > ?2
                AND m.sender_kind = 'bot' AND m.sender_bot_id = c.bot_id",
            params![bot_id, read],
            |r| r.get(0),
        )?)
    }

    /// Marks the thread read up to `up_to_num`; read state never goes back.
    /// The stored position.
    pub fn mark_owner_read(&self, bot_id: &str, up_to_num: i64) -> anyhow::Result<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO owner_read(bot_id, last_read_num, read_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(bot_id) DO UPDATE SET
               last_read_num = max(last_read_num, excluded.last_read_num),
               read_at = excluded.read_at",
            params![bot_id, up_to_num.max(0), ts(now())],
        )?;
        Ok(conn.query_row(
            "SELECT last_read_num FROM owner_read WHERE bot_id = ?1",
            params![bot_id],
            |r| r.get(0),
        )?)
    }

    pub fn add_owner_question(
        &self,
        bot: &bus::Bot,
        asked: Asked<'_>,
        title: &str,
    ) -> anyhow::Result<OwnerQuestion> {
        let (message_num, item_id) = match asked {
            Asked::Thread(num) => (Some(num), None),
            Asked::Card(item_id) => (None, Some(item_id.to_string())),
        };
        let question = OwnerQuestion {
            id: new_id(),
            bot_id: bot.id.clone(),
            project_id: bot.project_id.clone(),
            message_num,
            item_id,
            title: title.to_string(),
            created_at: now(),
        };
        self.lock().execute(
            "INSERT INTO owner_question(id, bot_id, project_id, message_num, item_id, title, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                question.id,
                question.bot_id,
                question.project_id,
                question.message_num,
                question.item_id,
                question.title,
                ts(question.created_at)
            ],
        )?;
        Ok(question)
    }

    /// Questions not dismissed, of a project or a bot, oldest first. A
    /// thread question the owner has since written in is left out; a card
    /// question's card is the caller's to check.
    pub fn open_owner_questions(
        &self,
        project_id: Option<&str>,
        bot_id: Option<&str>,
    ) -> anyhow::Result<Vec<OwnerQuestion>> {
        let sql = format!(
            "SELECT {QUESTION_COLS} FROM owner_question q
              WHERE q.dismissed_at IS NULL
                AND (?1 IS NULL OR q.project_id = ?1) AND (?2 IS NULL OR q.bot_id = ?2)
                AND (q.message_num IS NULL OR NOT EXISTS (
                     SELECT 1 FROM message m JOIN conversation c ON c.id = m.conversation_id
                      WHERE c.bot_id = q.bot_id AND m.num > q.message_num AND {FROM_OWNER}))
              ORDER BY q.created_at, q.rowid"
        );
        let conn = self.lock();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![project_id, bot_id], question_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The thread messages of the bot that asked, as `(num, question id)`.
    pub fn thread_questions(&self, bot_id: &str) -> anyhow::Result<Vec<(i64, String)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT message_num, id FROM owner_question
              WHERE bot_id = ?1 AND message_num IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![bot_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn owner_question(&self, id: &str) -> anyhow::Result<Option<OwnerQuestion>> {
        let sql = format!(
            "SELECT {QUESTION_COLS} FROM owner_question q
              WHERE q.id = ?1 AND q.dismissed_at IS NULL"
        );
        Ok(self
            .lock()
            .query_row(&sql, params![id], question_from_row)
            .optional()?)
    }

    /// Closes the question for the owner; `false` when it was already.
    pub fn dismiss_owner_question(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self.lock().execute(
            "UPDATE owner_question SET dismissed_at = ?2 WHERE id = ?1 AND dismissed_at IS NULL",
            params![id, ts(now())],
        )? > 0)
    }

    /// Whether the owner commented on the card after `since`.
    pub fn owner_commented_since(
        &self,
        item_id: &str,
        since: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        Ok(self.lock().query_row(
            "SELECT EXISTS (SELECT 1 FROM item_comment
              WHERE item_id = ?1 AND at > ?2 AND (author = 'user' OR author LIKE 'device:%'))",
            params![item_id, ts(since)],
            |r| r.get(0),
        )?)
    }

    /// The peers a bot that runs here is linked to.
    pub fn peers_exposed_to(&self, bot_id: &str) -> anyhow::Result<Vec<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT peer_id FROM peer_link WHERE bot_id = ?1")?;
        let rows = stmt.query_map(params![bot_id], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // ---- pins (D7) ----

    /// Pins or unpins the project here; `true` when that changed it.
    pub fn set_project_pinned(&self, project_id: &str, pinned: bool) -> anyhow::Result<bool> {
        let changed = if pinned {
            self.lock().execute(
                "INSERT OR IGNORE INTO project_pin(project_id, pinned_at) VALUES (?1, ?2)",
                params![project_id, ts(now())],
            )?
        } else {
            self.lock().execute(
                "DELETE FROM project_pin WHERE project_id = ?1",
                params![project_id],
            )?
        };
        Ok(changed > 0)
    }

    pub fn project_pinned(&self, project_id: &str) -> anyhow::Result<bool> {
        Ok(self.lock().query_row(
            "SELECT EXISTS (SELECT 1 FROM project_pin WHERE project_id = ?1)",
            params![project_id],
            |r| r.get(0),
        )?)
    }
}
