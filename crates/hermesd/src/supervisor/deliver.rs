//! Handing a rendered envelope to a bot's live session.

use super::*;

impl Supervisor {
    /// Deliver a rendered envelope through the session's inbox socket. The
    /// message is read between tool calls or starts a new turn when the
    /// session is idle; it never touches the terminal.
    pub fn deliver(&self, bot_id: &str, text: &str) -> Result<(), DeliverError> {
        let (session, socket) = {
            let bots = self.lock_bots();
            let Some(h) = bots.get(bot_id) else {
                return Err(DeliverError::NotReady("bot has no runtime".to_string()));
            };
            // A session being stopped still holds the inbox it is closing
            // until its exit is handled: a message sent there is lost with it
            // or fails into a retry backoff, instead of waiting for the
            // session that replaces it (H-188).
            if h.session.is_none() || h.stopping || !h.state.is_running() {
                return Err(DeliverError::NotReady(format!(
                    "bot is {}",
                    h.state.as_str()
                )));
            }
            (h.session.clone(), h.msg_socket.clone())
        };
        if let Some(session) = session {
            if let Some(result) = session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .deliver(text)
            {
                return result.map_err(DeliverError::Failed);
            }
        }
        let socket = socket
            .ok_or_else(|| DeliverError::NotReady("inbox socket not reported yet".to_string()))?;
        crate::channel::send(&socket, text).map_err(DeliverError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overrides::AutoCompactOverride;
    use crate::runtime::double::DoubleAdapter;

    /// H-188: between a restart's kill and the old session's exit being
    /// handled, a message waits for the new session rather than going to the
    /// inbox being closed.
    #[tokio::test]
    async fn a_session_being_restarted_takes_no_message() {
        let home = tempfile::tempdir().expect("tempdir");
        let cfg = Config {
            home: home.path().to_path_buf(),
            user_home: home.path().join("user"),
            ..Config::default()
        };
        let db = Db::open(&home.path().join("bus.sqlite")).expect("db");
        let project_id = db.create_project("p", "p").expect("project").id;
        let workspace = home.path().join("projects/p/bots/dev/workspace");
        let bot_id = db
            .create_bot_with_runtime(
                &project_id,
                "dev",
                "",
                "",
                "",
                &workspace.display().to_string(),
                "dev",
                None,
                bus::BotRuntime::ClaudeCode,
            )
            .expect("bot")
            .id;
        let secrets = Arc::new(Secrets::open(&home.path().join("secrets")).expect("secrets"));
        let sup = Supervisor::new(
            Arc::new(DoubleAdapter),
            cfg,
            db,
            Events::new(),
            secrets,
            AutoCompactOverride::default(),
        );
        sup.start_bot(&bot_id).expect("start");
        sup.deliver(&bot_id, "before")
            .expect("a live session takes it");

        // No await: the old session's exit has not been handled yet.
        sup.restart_bot(&bot_id).expect("restart");
        let sent = sup.deliver(&bot_id, "during");
        assert!(matches!(sent, Err(DeliverError::NotReady(_))), "{sent:?}");
    }
}
