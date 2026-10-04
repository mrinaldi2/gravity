//! Codex usage from App Server notifications (H-023 G2). The App Server
//! reports a thread's cumulative tokens in `thread/tokenUsage/updated` and
//! the account's windows in `account/rateLimits/updated`; both arrive over
//! the connection `runtime::codex` already holds, so nothing is tailed.
//!
//! Tokens are counted as the growth of the thread's running total over
//! what was counted before, kept in `usage_cursor` under the thread id: a
//! repeated update, or a resumed thread replaying its total, adds nothing.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use bus::Bot;

use crate::chat::usage_lines::Tokens;
use crate::db::{Db, ProviderWindow, UsageCursor, UsageMinute};

use super::limits::{self, WindowReading};
use super::prices::UsageConfig;

pub const PROVIDER: &str = "codex";

/// What one App Server notification tells the usage ledger.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    /// A thread's running token totals after a model response.
    Tokens {
        thread_id: String,
        model: String,
        total: Tokens,
        last: Tokens,
    },
    /// The account's windows. Sparse: a window the update leaves out keeps
    /// its last reading. `reached` is set once the account is out of
    /// allowance (`rateLimitReachedType`).
    Limits {
        windows: Vec<WindowReading>,
        reached: bool,
    },
    /// A turn failed because the account is out of allowance.
    LimitReached,
}

/// The ledger's reading of an App Server notification, or `None` for one
/// that carries no usage. `model` is the thread's current model, which
/// token updates do not name.
pub fn parse(method: &str, params: &Value, model: &str) -> Option<Report> {
    match method {
        "thread/tokenUsage/updated" => {
            let usage = params.get("tokenUsage")?;
            Some(Report::Tokens {
                thread_id: params.get("threadId")?.as_str()?.to_string(),
                model: model.to_string(),
                total: tokens(usage.get("total")?),
                last: tokens(usage.get("last")?),
            })
        }
        "account/rateLimits/updated" => {
            let snapshot = params.get("rateLimits")?;
            // Other limit ids meter a single model's alias, not the
            // account's pool.
            if snapshot
                .get("limitId")
                .and_then(Value::as_str)
                .is_some_and(|id| id != PROVIDER)
            {
                return None;
            }
            let windows: Vec<WindowReading> = ["primary", "secondary"]
                .iter()
                .filter_map(|key| reading(snapshot.get(*key)?))
                .collect();
            let reached = snapshot
                .get("rateLimitReachedType")
                .is_some_and(|t| !t.is_null());
            (!windows.is_empty() || reached).then_some(Report::Limits { windows, reached })
        }
        "error" | "turn/completed" => {
            let info = params
                .pointer("/error/codexErrorInfo")
                .or_else(|| params.pointer("/turn/error/codexErrorInfo"))?
                .as_str()?;
            (info == "usageLimitExceeded").then_some(Report::LimitReached)
        }
        _ => None,
    }
}

/// App Server breakdowns count cached and cache-write tokens inside
/// `inputTokens`, and reasoning inside `outputTokens`.
fn tokens(breakdown: &Value) -> Tokens {
    let n = |key: &str| {
        breakdown
            .get(key)
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0)
    };
    let cache_read = n("cachedInputTokens");
    let cache_write = n("cacheWriteInputTokens");
    Tokens {
        input: (n("inputTokens") - cache_read - cache_write).max(0),
        cache_write,
        cache_write_1h: 0,
        cache_read,
        output: n("outputTokens"),
        reasoning: n("reasoningOutputTokens"),
    }
}

/// A window is known by its length, not its slot: an account without a
/// 5-hour window reports the weekly one as `primary`.
fn reading(window: &Value) -> Option<WindowReading> {
    let name = match window.get("windowDurationMins")?.as_i64()? {
        300 => "5h",
        10080 => "weekly",
        _ => return None,
    };
    Some(WindowReading {
        window: name,
        used_percent: window.get("usedPercent")?.as_f64()?,
        resets_at: window
            .get("resetsAt")
            .and_then(Value::as_i64)
            .and_then(|s| Utc.timestamp_opt(s, 0).single()),
    })
}

/// Writes `report`, received at `at`, into the ledger. Returns whether it
/// put the Codex pool on hold: the account hit a limit that resets later.
pub fn record(
    db: &Db,
    cfg: &UsageConfig,
    bot: &Bot,
    report: &Report,
    at: DateTime<Utc>,
) -> anyhow::Result<bool> {
    match report {
        Report::Tokens {
            thread_id,
            model,
            total,
            last,
        } => {
            record_tokens(db, cfg, bot, thread_id, model, total, last, at)?;
            Ok(false)
        }
        Report::Limits { windows, reached } => {
            for w in windows {
                db.put_provider_window(&ProviderWindow {
                    provider: PROVIDER.to_string(),
                    window: w.window.to_string(),
                    used_percent: Some(w.used_percent),
                    resets_at: w.resets_at,
                    source: "observed".to_string(),
                    capacity_estimate: None,
                    limited_until: None,
                    updated_at: at,
                })?;
            }
            if *reached {
                limits::limit_full_windows(db, PROVIDER, at)
            } else {
                Ok(false)
            }
        }
        Report::LimitReached => limits::limit_full_windows(db, PROVIDER, at),
    }
}

#[allow(clippy::too_many_arguments)]
fn record_tokens(
    db: &Db,
    cfg: &UsageConfig,
    bot: &Bot,
    thread_id: &str,
    model: &str,
    total: &Tokens,
    last: &Tokens,
    at: DateTime<Utc>,
) -> anyhow::Result<()> {
    let key = format!("codex-thread:{thread_id}");
    let cursor = db.usage_cursor(&key)?;
    let counted: Option<Tokens> = cursor
        .as_ref()
        .and_then(|c| serde_json::from_str(&c.seen_json).ok());
    let added = match counted {
        None => *last,
        // A total that went backwards belongs to a thread that started
        // over; only the latest response is new.
        Some(seen) if total.input + total.cache_read < seen.input + seen.cache_read => *last,
        Some(seen) => total.over(&seen),
    };
    let rows = if added.is_zero() {
        Vec::new()
    } else {
        vec![UsageMinute {
            bot_id: bot.id.clone(),
            minute: super::ledger::minute(at),
            provider: PROVIDER.to_string(),
            model: model.to_string(),
            project_id: bot.project_id.clone(),
            input: added.input,
            cache_write: added.cache_write,
            cache_read: added.cache_read,
            output: added.output,
            reasoning: added.reasoning,
            units: cfg.price(model).map_or(0.0, |p| p.units(&added)),
            machine: None,
        }]
    };
    let next = UsageCursor {
        offset: (total.input + total.cache_read + total.cache_write + total.output).max(0) as u64,
        seen_json: serde_json::to_string(total)?,
    };
    db.record_usage(&rows, &bot.id, &key, &next)
}
