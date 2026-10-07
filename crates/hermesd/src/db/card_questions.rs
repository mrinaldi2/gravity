//! Card questions (H-211). On the board's home, `card_question` keeps each
//! bot's question comment, so the owner's answer reaches every bot that
//! asked, wherever it runs. On the asking bot's computer,
//! `owner_question_comment` keeps which comment its question is.

use rusqlite::{params, OptionalExtension};

use crate::board::model::ItemComment;

use super::{ts, Db};

/// A bot that asked on a card the owner just commented on.
#[derive(Debug, Clone, PartialEq)]
pub struct Asker {
    pub bot_id: String,
    /// The owner's comment replies to this bot's question.
    pub replied: bool,
}

/// An owner's comment, as stored.
const BY_OWNER: &str = "(m.author = 'user' OR m.author LIKE 'device:%')";

impl Db {
    /// `comment` is `bot_id`'s question for the owner.
    pub fn add_card_question(&self, comment: &ItemComment, bot_id: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT OR IGNORE INTO card_question(comment_id, item_id, bot_id, at)
             VALUES (?1, ?2, ?3, ?4)",
            params![comment.id, comment.item_id, bot_id, ts(comment.at)],
        )?;
        Ok(())
    }

    /// The bots whose question on `comment`'s card it answers: every
    /// question no earlier owner comment answered, and the one it replies to.
    pub fn askers_answered_by(&self, comment: &ItemComment) -> anyhow::Result<Vec<Asker>> {
        let sql = format!(
            "SELECT q.bot_id, max(q.comment_id = ?3) FROM card_question q
              WHERE q.item_id = ?1 AND q.at <= ?2
                AND (q.comment_id = ?3 OR NOT EXISTS (
                     SELECT 1 FROM item_comment m
                      WHERE m.item_id = q.item_id AND m.at > q.at AND m.at < ?2
                        AND m.id != ?4 AND {BY_OWNER}))
              GROUP BY q.bot_id ORDER BY min(q.at)"
        );
        let conn = self.lock();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![
                comment.item_id,
                ts(comment.at),
                comment.reply_to.as_deref().unwrap_or_default(),
                comment.id
            ],
            |r| {
                Ok(Asker {
                    bot_id: r.get(0)?,
                    replied: r.get(1)?,
                })
            },
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Records the comment a card question is.
    pub fn set_question_comment(&self, question_id: &str, comment_id: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT OR REPLACE INTO owner_question_comment(question_id, comment_id)
             VALUES (?1, ?2)",
            params![question_id, comment_id],
        )?;
        Ok(())
    }

    /// The comment a card question is, when it was recorded.
    pub fn question_comment(&self, question_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT comment_id FROM owner_question_comment WHERE question_id = ?1",
                params![question_id],
                |r| r.get(0),
            )
            .optional()?)
    }
}
