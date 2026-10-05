//! Startup safety nets for Claude Code sessions.
//!
//! A Claude Code bot is only reachable once its SessionStart hook has reported
//! the session's inbox socket. When that report is lost (the daemon was not
//! serving yet, or the session sat at a trust dialog), the bot looks "ready"
//! while every delivery to it stays queued. Three guards keep that from
//! lasting:
//!
//! - the connect watchdog restarts a session that has not reported its socket
//!   in time, a bounded number of times, then says so to the owner;
//! - mass starts are staggered, so a daemon boot does not launch every
//!   session at once into a fight over Claude Code's config lock;
//! - a trust pre-approval that loses that fight defers the start and retries
//!   with backoff, instead of launching the session into the trust dialog.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::*;

/// The state reason shown once the watchdog has given up on a bot.
pub const DIDNT_CONNECT: &str = "Didn't connect — Restart bot";
/// Trust attempts deferred for a busy config lock before starting anyway.
const TRUST_RETRIES: u32 = 4;
const TRUST_RETRY_BASE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupConfig {
    /// How long a Claude Code session may run without reporting its inbox
    /// socket before the watchdog restarts it.
    pub connect_timeout_ms: u64,
    /// Watchdog restarts per bot before it gives up and asks the owner.
    pub connect_restarts: u32,
    /// Claude Code sessions allowed to be coming up at once.
    pub max_concurrent_starts: usize,
    /// How long a started session counts as coming up when it never reports
    /// its socket, so a stuck one cannot hold a start slot forever.
    pub warmup_ms: u64,
}

impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 90_000,
            connect_restarts: 2,
            max_concurrent_starts: 6,
            warmup_ms: 15_000,
        }
    }
}

/// What the watchdog decided for one bot on this tick.
enum Verdict {
    Restart(u32),
    GiveUp,
}

impl BotHandle {
    /// A Claude Code session that is up but has not reported its socket yet.
    fn awaiting_socket(&self) -> bool {
        self.session.is_some()
            && self.msg_socket.is_none()
            && self.terminal_runtime == Some(bus::BotRuntime::ClaudeCode)
    }
}

impl Supervisor {
    fn startup(&self) -> &StartupConfig {
        &self.inner.cfg.startup
    }

    /// True when as many Claude Code sessions are coming up as the stagger
    /// allows; the reconciler leaves further Claude starts for a later tick.
    pub(super) fn start_slots_full(&self) -> bool {
        let warmup = Duration::from_millis(self.startup().warmup_ms);
        let warming = self
            .lock_bots()
            .values()
            .filter(|h| h.starting || (h.awaiting_socket() && h.last_start.elapsed() < warmup))
            .count();
        warming >= self.startup().max_concurrent_starts.max(1)
    }

    /// Restart sessions that never reported their inbox socket. Runs on every
    /// supervision tick.
    pub(super) fn watch_connections(&self) {
        let timeout = Duration::from_millis(self.startup().connect_timeout_ms);
        let limit = self.startup().connect_restarts;
        let verdicts: Vec<(String, Verdict)> = {
            let mut bots = self.lock_bots();
            bots.iter_mut()
                .filter(|(_, h)| {
                    h.awaiting_socket()
                        && !h.stopping
                        && !h.starting
                        && !h.connect_gave_up
                        // A turn in flight is left to finish: the owner may be
                        // typing into the terminal.
                        && matches!(h.state, BotState::Starting | BotState::Ready)
                        && h.last_start.elapsed() >= timeout
                })
                .map(|(id, h)| {
                    if h.connect_restarts < limit {
                        h.connect_restarts += 1;
                        (id.clone(), Verdict::Restart(h.connect_restarts))
                    } else {
                        h.connect_gave_up = true;
                        (id.clone(), Verdict::GiveUp)
                    }
                })
                .collect()
        };
        for (bot_id, verdict) in verdicts {
            match verdict {
                Verdict::Restart(attempt) => {
                    tracing::warn!(
                        bot_id,
                        attempt,
                        limit,
                        waited_ms = timeout.as_millis() as u64,
                        "inbox socket never registered; restarting the session"
                    );
                    if let Err(e) = self.restart_session(&bot_id) {
                        tracing::warn!(bot_id, error = %e, "watchdog restart failed");
                    }
                }
                Verdict::GiveUp => {
                    tracing::warn!(
                        bot_id,
                        restarts = limit,
                        "inbox socket never registered; giving up until the owner restarts the bot"
                    );
                    self.set_state(&bot_id, BotState::WaitingForUser, DIDNT_CONNECT);
                    self.inner.events.push(Push::notice(
                        "error",
                        format!("{} didn't connect", self.bot_name(&bot_id)),
                        "Messages can't reach it. Restart bot to try again.",
                    ));
                }
            }
        }
    }

    /// The socket arrived: the bot is reachable, and the watchdog starts over.
    pub(super) fn on_connected(&self, bot_id: &str) {
        let gave_up = {
            let mut bots = self.lock_bots();
            let Some(h) = bots.get_mut(bot_id) else {
                return;
            };
            h.connect_restarts = 0;
            std::mem::take(&mut h.connect_gave_up)
        };
        if gave_up && self.state(bot_id) == (BotState::WaitingForUser, DIDNT_CONNECT.to_string()) {
            self.set_state(bot_id, BotState::Ready, "inbox socket registered");
        }
    }

    /// The owner asked for a restart: the watchdog gets its full budget back.
    pub(super) fn reset_watchdog(&self, bot_id: &str) {
        if let Some(h) = self.lock_bots().get_mut(bot_id) {
            h.connect_restarts = 0;
            h.connect_gave_up = false;
        }
    }

    /// Pre-approve the workspace in Claude Code's trust list. False when the
    /// config lock was busy and the start is deferred to retry with backoff;
    /// any other failure, or a lock that stays busy, starts the bot anyway.
    pub(super) fn trust_or_defer(&self, bot_id: &str, workspace: &Path) -> bool {
        let Err(e) = crate::paths::trust_workspace(&self.inner.cfg.user_home, workspace) else {
            if let Some(h) = self.lock_bots().get_mut(bot_id) {
                h.trust_retries = 0;
            }
            return true;
        };
        let busy = e.downcast_ref::<crate::paths::ConfigLocked>().is_some();
        let retry = {
            let mut bots = self.lock_bots();
            let h = bots
                .entry(bot_id.to_string())
                .or_insert_with(|| BotHandle::new(self.inner.cfg.scrollback_bytes));
            if busy && h.trust_retries < TRUST_RETRIES {
                let delay = TRUST_RETRY_BASE * 2u32.pow(h.trust_retries);
                h.trust_retries += 1;
                h.next_start_at = Some(Instant::now() + delay);
                Some((h.trust_retries, delay))
            } else {
                h.trust_retries = 0;
                None
            }
        };
        match retry {
            Some((attempt, delay)) => {
                tracing::warn!(
                    bot_id,
                    attempt,
                    retry_in_ms = delay.as_millis() as u64,
                    error = %e,
                    "could not trust bot workspace; deferring the start"
                );
                false
            }
            None => {
                tracing::warn!(bot_id, error = %e, "could not trust bot workspace");
                true
            }
        }
    }
}

#[cfg(test)]
#[path = "watchdog_tests.rs"]
mod tests;
