//! The usage ledger (migration 24): per-minute token totals per bot, how
//! far each transcript has been counted, and each provider's account
//! windows. See [`crate::usage`].

use chrono::{DateTime, Utc};
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

/// One account window of a provider (`5h` or `weekly`), as last observed or
/// estimated.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderWindow {
    pub provider: String,
    pub window: String,
    pub used_percent: Option<f64>,
    pub resets_at: Option<DateTime<Utc>>,
    /// `observed` (the provider said so), `estimated` (computed from units
    /// and a calibrated capacity) or `reported` (a peer's figure).
    pub source: String,
    /// Cost units the window holds, calibrated at observed readings.
    pub capacity_estimate: Option<f64>,
    /// Set when a bot hit this window's limit: the pool waits until then.
    pub limited_until: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
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

    pub fn provider_window(
        &self,
        provider: &str,
        window: &str,
    ) -> anyhow::Result<Option<ProviderWindow>> {
        Ok(self
            .provider_windows(provider)?
            .into_iter()
            .find(|w| w.window == window))
    }

    /// A provider's windows, `5h` before `weekly`.
    pub fn provider_windows(&self, provider: &str) -> anyhow::Result<Vec<ProviderWindow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT provider, window, used_percent, resets_at, source, capacity_estimate,
                    limited_until, updated_at
             FROM provider_window WHERE provider = ?1 ORDER BY window",
        )?;
        let rows = stmt
            .query_map([provider], |r| {
                Ok(ProviderWindow {
                    provider: r.get(0)?,
                    window: r.get(1)?,
                    used_percent: r.get(2)?,
                    resets_at: r.get::<_, Option<String>>(3)?.map(|s| super::parse_ts(&s)),
                    source: r.get(4)?,
                    capacity_estimate: r.get(5)?,
                    limited_until: r.get::<_, Option<String>>(6)?.map(|s| super::parse_ts(&s)),
                    updated_at: super::parse_ts(&r.get::<_, String>(7)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Writes a window's reading. The capacity estimate and the limit are
    /// kept unless `w` carries them: a reading alone never forgets the
    /// calibration, nor lifts a limit a bot hit.
    pub fn put_provider_window(&self, w: &ProviderWindow) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT INTO provider_window(provider, window, used_percent, resets_at, source,
                 capacity_estimate, limited_until, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(provider, window) DO UPDATE SET
                 used_percent = excluded.used_percent, resets_at = excluded.resets_at,
                 source = excluded.source,
                 capacity_estimate = COALESCE(excluded.capacity_estimate, capacity_estimate),
                 limited_until = COALESCE(excluded.limited_until, limited_until),
                 updated_at = excluded.updated_at",
            params![
                w.provider,
                w.window,
                w.used_percent,
                w.resets_at.map(super::ts),
                w.source,
                w.capacity_estimate,
                w.limited_until.map(super::ts),
                super::ts(w.updated_at),
            ],
        )?;
        Ok(())
    }

    /// Cost units every bot of `provider` used from `since` (a stored
    /// minute) on, on this machine and reported by peers.
    pub fn usage_units_since(&self, provider: &str, since: &str) -> anyhow::Result<f64> {
        let units = self.lock().query_row(
            "SELECT COALESCE(SUM(units), 0) FROM usage_minute
             WHERE provider = ?1 AND minute >= ?2",
            params![provider, since],
            |r| r.get(0),
        )?;
        Ok(units)
    }
}
