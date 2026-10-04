//! Token usage per bot, counted from runtime transcripts into
//! `usage_minute` (H-023 G1). Each scan reads what every local Claude Code
//! bot appended since the last one; the chat pane's tailer only follows
//! chats someone has open, so this keeps its own cursors.
//!
//! Codex bots report their tokens and the account's windows live, through
//! their App Server connection; see [`codex`] (G2). Account limits, and
//! the hold on a pool that hit one, are in [`limits`] (G3).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bus::{Bot, BotRuntime};
use chrono::Utc;

use crate::app::AppState;

pub mod codex;
mod ledger;
pub mod limits;
mod prices;
#[cfg(test)]
mod tests;

pub use ledger::{ingest_file, Ingested};
pub use prices::{ModelPrice, UsageConfig};

/// A transcript nobody counted yet is only read if it was written this
/// recently: one weekly window plus a day, not a bot's whole history.
const LOOKBACK: Duration = Duration::from_secs(8 * 24 * 3600);

/// How long an archived bot is still scanned, so a worker's last turn is
/// counted after its task closes.
const ARCHIVED_GRACE_HOURS: i64 = 24;

pub async fn watch(app: Arc<AppState>) {
    if !app.cfg.usage.enabled {
        return;
    }
    let every = Duration::from_secs(app.cfg.usage.interval_seconds.max(1));
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let app = app.clone();
        let _ = tokio::task::spawn_blocking(move || {
            scan(&app);
            app.supervisor.apply_pool_limits();
        })
        .await;
    }
}

/// Counts what every local Claude Code bot used since the last scan, then
/// records the limit hits it found and re-estimates the Claude windows.
pub fn scan(app: &AppState) {
    let Ok(bots) = app.db.list_bots_with_archived(None) else {
        return;
    };
    let cutoff = Utc::now() - chrono::Duration::hours(ARCHIVED_GRACE_HOURS);
    let mut hits = Vec::new();
    for bot in bots {
        if bot.is_linked()
            || bot.runtime != BotRuntime::ClaudeCode
            || bot.deleted_at.is_some_and(|at| at < cutoff)
        {
            continue;
        }
        for path in transcripts(app, &bot) {
            match ingest_file(&app.db, &app.cfg.usage, &bot, &path) {
                Ok(found) => hits.extend(found.hits),
                Err(e) => {
                    tracing::debug!(bot_id = bot.id, path = %path.display(), error = %e, "usage scan failed")
                }
            }
        }
    }
    let now = Utc::now();
    for hit in &hits {
        match limits::record_hit(&app.db, hit, now) {
            Ok(true) => tracing::info!(
                provider = hit.provider,
                window = hit.window,
                resets_at = %hit.resets_at,
                "account limit hit"
            ),
            Ok(false) => {}
            Err(e) => tracing::debug!(error = %e, "recording a limit hit failed"),
        }
    }
    if let Err(e) = limits::estimate(&app.db, now) {
        tracing::debug!(error = %e, "estimating Claude windows failed");
    }
}

/// The bot's transcripts that may hold uncounted usage.
fn transcripts(app: &AppState, bot: &Bot) -> Vec<PathBuf> {
    let dir = crate::activity::transcript_dir(&app.cfg.user_home, Path::new(&bot.workspace_path));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let oldest = SystemTime::now() - LOOKBACK;
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .filter(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .is_ok_and(|t| t >= oldest)
        })
        .collect()
}
