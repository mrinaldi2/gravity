//! The usage ledger (migration 24): per-minute token totals per bot, and
//! how far each transcript has been counted. See [`crate::usage`].

use rusqlite::{params, OptionalExtension};

use super::Db;

/// Tokens and cost units one bot used in one minute on one model.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageMinute {
    pub bot_id: String,
    /// RFC 3339, truncated to the minute, UTC.
    pub minute: String,
    pub provider: String,
    pub model: String,
    pub project_id: String,
    pub input: i64,
    /// All cache writes, whatever their lifetime.
    pub cache_write: i64,
    pub cache_read: i64,
    pub output: i64,
    pub reasoning: i64,
    pub units: f64,
    pub machine: Option<String>,
}

/// Where counting stopped in one transcript.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageCursor {
    pub offset: u64,
    /// Recently counted message ids and what was counted for each.
    pub seen_json: String,
}

impl Db {
    pub fn usage_cursor(&self, path: &str) -> anyhow::Result<Option<UsageCursor>> {
        let cursor = self
            .lock()
            .query_row(
                "SELECT offset, seen_json FROM usage_cursor WHERE path = ?1",
                [path],
                |r| {
                    Ok(UsageCursor {
                        offset: r.get::<_, i64>(0)?.max(0) as u64,
                        seen_json: r.get(1)?,
                    })
                },
            )
            .optional()?;
        Ok(cursor)
    }

    /// Adds `rows` onto the minutes they fall in and moves the transcript's
    /// cursor, in one transaction: a crash counts a stretch fully or not at
    /// all.
    pub fn record_usage(
        &self,
        rows: &[UsageMinute],
        bot_id: &str,
        path: &str,
        cursor: &UsageCursor,
    ) -> anyhow::Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for row in rows {
            tx.execute(
                "INSERT INTO usage_minute(bot_id, minute, provider, model, project_id, input,
                     cache_write, cache_read, output, reasoning, units, machine)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(bot_id, minute, provider, model) DO UPDATE SET
                     input = input + excluded.input,
                     cache_write = cache_write + excluded.cache_write,
                     cache_read = cache_read + excluded.cache_read,
                     output = output + excluded.output,
                     reasoning = reasoning + excluded.reasoning,
                     units = units + excluded.units",
                params![
                    row.bot_id,
                    row.minute,
                    row.provider,
                    row.model,
                    row.project_id,
                    row.input,
                    row.cache_write,
                    row.cache_read,
                    row.output,
                    row.reasoning,
                    row.units,
                    row.machine,
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO usage_cursor(path, bot_id, offset, seen_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
                 bot_id = excluded.bot_id, offset = excluded.offset,
                 seen_json = excluded.seen_json, updated_at = excluded.updated_at",
            params![
                path,
                bot_id,
                cursor.offset as i64,
                cursor.seen_json,
                super::ts(bus::now())
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// A bot's minutes from `since` (inclusive), oldest first.
    pub fn usage_minutes(&self, bot_id: &str, since: &str) -> anyhow::Result<Vec<UsageMinute>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT bot_id, minute, provider, model, project_id, input, cache_write,
                    cache_read, output, reasoning, units, machine
             FROM usage_minute WHERE bot_id = ?1 AND minute >= ?2
             ORDER BY minute, model",
        )?;
        let rows = stmt
            .query_map(params![bot_id, since], |r| {
                Ok(UsageMinute {
                    bot_id: r.get(0)?,
                    minute: r.get(1)?,
                    provider: r.get(2)?,
                    model: r.get(3)?,
                    project_id: r.get(4)?,
                    input: r.get(5)?,
                    cache_write: r.get(6)?,
                    cache_read: r.get(7)?,
                    output: r.get(8)?,
                    reasoning: r.get(9)?,
                    units: r.get(10)?,
                    machine: r.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
