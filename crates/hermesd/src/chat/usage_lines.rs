//! Token usage read from a Claude Code transcript line, for the usage
//! ledger (see [`crate::usage`]). Every `assistant` line carries the usage
//! of the API message it belongs to; a message streamed as several content
//! blocks repeats it on each line, so callers dedupe by `message_id`.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Tokens one API message used. `reasoning` is the thinking share of
/// `output`, not an addition to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tokens {
    pub input: i64,
    /// Cache writes at the 5-minute rate.
    pub cache_write: i64,
    /// Cache writes at the 1-hour rate.
    pub cache_write_1h: i64,
    pub cache_read: i64,
    pub output: i64,
    pub reasoning: i64,
}

impl Tokens {
    pub fn is_zero(&self) -> bool {
        *self == Tokens::default()
    }

    /// What `self` adds over `seen`, field by field, never negative: a later
    /// copy of a message can only grow its counts.
    pub fn over(&self, seen: &Tokens) -> Tokens {
        let d = |a: i64, b: i64| (a - b).max(0);
        Tokens {
            input: d(self.input, seen.input),
            cache_write: d(self.cache_write, seen.cache_write),
            cache_write_1h: d(self.cache_write_1h, seen.cache_write_1h),
            cache_read: d(self.cache_read, seen.cache_read),
            output: d(self.output, seen.output),
            reasoning: d(self.reasoning, seen.reasoning),
        }
    }

    pub fn max(&self, other: &Tokens) -> Tokens {
        Tokens {
            input: self.input.max(other.input),
            cache_write: self.cache_write.max(other.cache_write),
            cache_write_1h: self.cache_write_1h.max(other.cache_write_1h),
            cache_read: self.cache_read.max(other.cache_read),
            output: self.output.max(other.output),
            reasoning: self.reasoning.max(other.reasoning),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageLine {
    pub message_id: String,
    pub model: String,
    pub at: DateTime<Utc>,
    pub tokens: Tokens,
}

/// The usage on one transcript line, if it is an assistant message that
/// used any tokens. Synthetic messages (API errors) carry none.
pub fn parse(line: &str) -> Option<UsageLine> {
    if !line.contains("\"usage\"") {
        return None;
    }
    let record: Value = serde_json::from_str(line).ok()?;
    if record.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let message = record.get("message")?;
    let usage = message.get("usage")?;
    let model = message.get("model")?.as_str()?;
    if model.starts_with('<') {
        return None;
    }
    let message_id = message.get("id")?.as_str()?.to_string();
    let at = record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())?
        .with_timezone(&Utc);
    let n = |v: Option<&Value>| v.and_then(Value::as_i64).unwrap_or(0).max(0);
    let cache_total = n(usage.get("cache_creation_input_tokens"));
    let split = usage.get("cache_creation");
    let cache_write_1h = n(split.and_then(|c| c.get("ephemeral_1h_input_tokens"))).min(cache_total);
    let tokens = Tokens {
        input: n(usage.get("input_tokens")),
        cache_write: cache_total - cache_write_1h,
        cache_write_1h,
        cache_read: n(usage.get("cache_read_input_tokens")),
        output: n(usage.get("output_tokens")),
        reasoning: n(usage
            .get("output_tokens_details")
            .and_then(|d| d.get("thinking_tokens"))),
    };
    (!tokens.is_zero()).then(|| UsageLine {
        message_id,
        model: model.to_string(),
        at,
        tokens,
    })
}
