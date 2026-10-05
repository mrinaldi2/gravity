//! Starting a bot's runtime session: its process tagged for the lineage
//! ledger (H-117 Q1) and its root recorded for bus identity (H-044).

use super::Supervisor;

impl Supervisor {
    /// Starts a session and records its root process as the bot's, replacing
    /// the last session's, so the old session's processes are cut off.
    pub(super) fn start_session(
        &self,
        bot_id: &str,
        spec: &crate::runtime::BotSpec,
    ) -> anyhow::Result<crate::runtime::StartedSession> {
        use crate::holders::ledger;
        // Every process the session starts inherits its tag (H-117 Q1).
        let session = bus::new_id();
        let mut spec = spec.clone();
        spec.env.push((
            crate::holders::procs::SESSION_ENV.to_string(),
            session.clone(),
        ));
        let started = self.inner.adapter.start(&spec)?;
        let project_id = self
            .inner
            .db
            .get_bot(bot_id)
            .ok()
            .flatten()
            .map(|b| b.project_id)
            .unwrap_or_default();
        let tag = ledger::SessionTag {
            project_id,
            bot_id: bot_id.to_string(),
        };
        ledger::session_started(&session, tag, started.session.root_pid());
        match started.session.root_pid() {
            Some(pid) => {
                let table = crate::bus_auth::os::OsProcessTable;
                self.inner.roots.record_pid(&table, pid, bot_id);
            }
            None => self.inner.roots.forget(bot_id),
        }
        Ok(started)
    }
}
