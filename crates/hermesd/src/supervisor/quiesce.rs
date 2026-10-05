//! Holding every bot still while this computer is quiesced (H-117 Q2).

use bus::BotState;

use super::Supervisor;

impl Supervisor {
    /// Stops every running session and shows `reason` on each bot, every
    /// tick of a pause, so a bot started by any other path is held again.
    /// Returns how many sessions it stopped.
    pub fn hold(&self, reason: &str, exempt: Option<&str>) -> usize {
        let (running, idle): (Vec<String>, Vec<String>) = {
            let bots = self.lock_bots();
            let running = bots
                .iter()
                .filter(|(id, _)| Some(id.as_str()) != exempt)
                .filter(|(_, h)| h.session.is_some() && !h.stopping)
                .map(|(id, _)| id.clone())
                .collect();
            let idle = bots
                .iter()
                .filter(|(id, _)| Some(id.as_str()) != exempt)
                .filter(|(_, h)| h.session.is_none() && !h.starting)
                .filter(|(_, h)| h.state != BotState::Stopped || h.reason != reason)
                .map(|(id, _)| id.clone())
                .collect();
            (running, idle)
        };
        for bot_id in &running {
            if let Err(e) = self.stop_for_pause(bot_id) {
                tracing::warn!(bot_id, error = %e, "quiesce could not stop a bot");
            }
        }
        for bot_id in &idle {
            self.show_paused(bot_id, reason);
        }
        running.len()
    }

    /// Stops the session as a restart does, so the bot may start again once
    /// the pause ends (a plain stop keeps a bot stopped for good); the
    /// supervision tick doesn't start it while the pause holds.
    fn stop_for_pause(&self, bot_id: &str) -> anyhow::Result<()> {
        let session = {
            let mut bots = self.lock_bots();
            let Some(h) = bots.get_mut(bot_id) else {
                return Ok(());
            };
            h.stopping = true;
            h.restart_pending = true;
            h.session.clone()
        };
        if let Some(session) = session {
            session.lock().unwrap_or_else(|e| e.into_inner()).kill()?;
        }
        Ok(())
    }

    /// `Stopped` with the pause's reason. `set_state` leaves a bot already
    /// stopped alone, reason and all, so this sets both itself.
    fn show_paused(&self, bot_id: &str, reason: &str) {
        {
            let mut bots = self.lock_bots();
            let Some(h) = bots.get_mut(bot_id) else {
                return;
            };
            h.state = BotState::Stopped;
            h.reason = reason.to_string();
        }
        self.inner.events.push(crate::events::Push::BotState {
            bot_id: bot_id.to_string(),
            state: BotState::Stopped,
            reason: reason.to_string(),
            at: chrono::Utc::now().to_rfc3339(),
        });
    }
}
