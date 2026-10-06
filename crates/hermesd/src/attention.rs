//! What needs the owner (H-128 §1): the one source of Needs-you rows, shared
//! by `dashboard_get`, `attention_rows` and `projects_overview`, so the count
//! on the projects home is the number of rows inside the project by
//! construction.
//!
//! Each computer builds the rows it owns (`Scope`): its own prompts, Run
//! cards, waiting bots, decisions and serving state, plus the board's
//! releases and P0s when the board lives here. Across computers the parts
//! are added up, never de-duplicated: nothing is owned twice.

use bus::contract::home::{AttentionKind, AttentionRow, AttentionSummary};
use bus::contract::pbjson_types::Timestamp;
use chrono::{DateTime, Utc};
use serde_json::Value;

mod legacy;
mod rows;
#[cfg(test)]
mod tests;

pub(crate) use legacy::needs_you;
pub(crate) use rows::{rows, Scope};

/// Each kind's weight in the score (H-128 §1). A product choice, kept in one
/// table so UX or the owner can tune it.
pub(crate) fn weight(kind: AttentionKind) -> u32 {
    match kind {
        AttentionKind::ReleaseAwaiting
        | AttentionKind::OwnerAction
        | AttentionKind::PermissionPrompt => 3,
        AttentionKind::P0Item | AttentionKind::ServingOff => 2,
        AttentionKind::RelayedRulings
        | AttentionKind::BotWaiting
        | AttentionKind::OffBoard
        | AttentionKind::OwnerQuestion => 1,
        // A decision's weight is its priority's (`decision_weight`).
        AttentionKind::Decision => 1,
        AttentionKind::Unspecified => 0,
    }
}

/// An urgent decision weighs as a P0 card, an ordinary one as the rest.
pub(crate) fn decision_weight(priority: bus::Priority) -> u32 {
    match priority {
        bus::Priority::Urgent => 3,
        bus::Priority::Normal => 1,
    }
}

/// The kind's name as `by_kind` keys and row ids carry it: lowercase.
pub(crate) fn kind_key(kind: AttentionKind) -> String {
    kind.as_str_name().to_ascii_lowercase()
}

/// A row id that stays the same across reads (iOS M3).
pub(crate) fn row_id(kind: AttentionKind, daemon_id: &str, target: &str) -> String {
    format!("{}:{daemon_id}:{target}", kind_key(kind))
}

/// One built row: the typed row, and the dashboard's shape of it for the
/// kinds the dashboard has always shown (`None` for the newer ones).
#[derive(Debug, Clone)]
pub(crate) struct Built {
    pub row: AttentionRow,
    pub legacy: Option<Value>,
}

/// The longest title a row carries (proto: 120 characters).
const TITLE_MAX: usize = 120;

/// `text` on one line, cut to `max` characters with "…".
pub(crate) fn cut(text: &str, max: usize) -> String {
    cut_text(text.lines().next().unwrap_or_default(), max)
}

/// `text` cut to `max` characters with "…", its lines kept.
pub(crate) fn cut_text(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// A row's title: one line, token-like values masked (CE-014 F1), cut.
pub(crate) fn title(text: &str) -> String {
    cut(
        &crate::redact::secrets(text.lines().next().unwrap_or_default()),
        TITLE_MAX,
    )
}

pub(crate) fn timestamp(at: DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: at.timestamp(),
        nanos: i32::try_from(at.timestamp_subsec_nanos()).unwrap_or(0),
    }
}

/// Sorts rows as `AttentionRows` lists them: weight desc, then oldest first,
/// then id, so equal rows keep one order on every read.
pub(crate) fn sort(rows: &mut [AttentionRow]) {
    rows.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| key(a.created_at.as_ref()).cmp(&key(b.created_at.as_ref())))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// A timestamp as a sortable key; none sorts last.
pub(crate) fn key(at: Option<&Timestamp>) -> (bool, i64, i32) {
    at.map_or((true, 0, 0), |t| (false, t.seconds, t.nanos))
}

/// `{count, score, by_kind, oldest_at}` of the rows (H-128 §1).
pub(crate) fn summary<'a>(rows: impl IntoIterator<Item = &'a AttentionRow>) -> AttentionSummary {
    let mut out = AttentionSummary::default();
    for row in rows {
        out.count += 1;
        out.score += row.weight;
        *out.by_kind.entry(kind_key(row.kind())).or_default() += 1;
        if key(row.created_at.as_ref()) < key(out.oldest_at.as_ref()) {
            out.oldest_at = row.created_at;
        }
    }
    out
}

/// Summaries added up, as the overview's `total` and a merged row are.
pub(crate) fn add(into: &mut AttentionSummary, part: &AttentionSummary) {
    into.count += part.count;
    into.score += part.score;
    for (kind, n) in &part.by_kind {
        *into.by_kind.entry(kind.clone()).or_default() += n;
    }
    if key(part.oldest_at.as_ref()) < key(into.oldest_at.as_ref()) {
        into.oldest_at = part.oldest_at;
    }
}
