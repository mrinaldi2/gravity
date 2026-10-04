//! Folding a transcript's new lines into per-minute rows.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use bus::Bot;
use chrono::{DateTime, Timelike, Utc};

use crate::chat::usage_lines::{self, Tokens};
use crate::db::{Db, UsageCursor, UsageMinute};

use super::limits::{self, LimitHit};
use super::prices::UsageConfig;

/// How many recent message ids a cursor remembers. A message's copies sit
/// next to each other in the transcript, so a short memory dedupes them
/// even when a scan, or a restart, falls between two copies.
const SEEN_IDS: usize = 64;

/// What one pass over a transcript found.
#[derive(Debug, Default)]
pub struct Ingested {
    /// New minute rows (or additions to existing ones) written.
    pub rows: usize,
    /// Limit hits among the new lines, for the caller to record once every
    /// bot's usage is in.
    pub hits: Vec<LimitHit>,
}

/// Counts the lines `path` gained since its cursor.
pub fn ingest_file(db: &Db, cfg: &UsageConfig, bot: &Bot, path: &Path) -> anyhow::Result<Ingested> {
    let key = path.to_string_lossy().to_string();
    let cursor = db.usage_cursor(&key)?.unwrap_or_default();
    let len = std::fs::metadata(path)?.len();
    if len == cursor.offset {
        return Ok(Ingested::default());
    }
    let mut seen = Seen::load(&cursor.seen_json);
    let mut minutes: BTreeMap<(String, String), (Tokens, f64)> = BTreeMap::new();
    let mut hits = Vec::new();
    let offset = crate::chat::read_from(path, cursor.offset, |_, line| {
        if let Some(hit) = limits::parse_claude_hit(line) {
            hits.push(hit);
            return;
        }
        let Some(usage) = usage_lines::parse(line) else {
            return;
        };
        let added = seen.count(&usage.message_id, usage.tokens);
        if added.is_zero() {
            return;
        }
        let units = cfg.price(&usage.model).map_or(0.0, |p| p.units(&added));
        let slot = minutes
            .entry((minute(usage.at), usage.model))
            .or_insert((Tokens::default(), 0.0));
        slot.0 = add(&slot.0, &added);
        slot.1 += units;
    })?;
    let rows: Vec<UsageMinute> = minutes
        .into_iter()
        .map(|((minute, model), (t, units))| UsageMinute {
            bot_id: bot.id.clone(),
            minute,
            provider: "claude".to_string(),
            model,
            project_id: bot.project_id.clone(),
            input: t.input,
            cache_write: t.cache_write + t.cache_write_1h,
            cache_read: t.cache_read,
            output: t.output,
            reasoning: t.reasoning,
            units,
            machine: None,
        })
        .collect();
    let next = UsageCursor {
        offset,
        seen_json: seen.dump(),
    };
    if offset != cursor.offset || !rows.is_empty() {
        db.record_usage(&rows, &bot.id, &key, &next)?;
    }
    Ok(Ingested {
        rows: rows.len(),
        hits,
    })
}

/// The start of `at`'s minute, as stored.
pub(super) fn minute(at: DateTime<Utc>) -> String {
    at.with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(at)
        .format("%Y-%m-%dT%H:%M:00Z")
        .to_string()
}

fn add(a: &Tokens, b: &Tokens) -> Tokens {
    Tokens {
        input: a.input + b.input,
        cache_write: a.cache_write + b.cache_write,
        cache_write_1h: a.cache_write_1h + b.cache_write_1h,
        cache_read: a.cache_read + b.cache_read,
        output: a.output + b.output,
        reasoning: a.reasoning + b.reasoning,
    }
}

/// Recently counted messages and the tokens counted for each.
struct Seen(VecDeque<(String, Tokens)>);

impl Seen {
    fn load(json: &str) -> Self {
        Self(serde_json::from_str(json).unwrap_or_default())
    }

    fn dump(&self) -> String {
        serde_json::to_string(&self.0).unwrap_or_else(|_| "[]".to_string())
    }

    /// What this copy of the message adds to what was already counted for
    /// it: everything the first time, then only growth.
    fn count(&mut self, id: &str, tokens: Tokens) -> Tokens {
        if let Some(entry) = self.0.iter_mut().find(|(seen, _)| seen == id) {
            let added = tokens.over(&entry.1);
            entry.1 = entry.1.max(&tokens);
            return added;
        }
        self.0.push_back((id.to_string(), tokens));
        while self.0.len() > SEEN_IDS {
            self.0.pop_front();
        }
        tokens
    }
}
