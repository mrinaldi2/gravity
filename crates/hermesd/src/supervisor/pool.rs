//! Account limits held fleet-wide per pool: once any bot of a provider hits
//! a window's limit, every local bot of that provider shows
//! `rate_limited` and receives nothing until the window resets (H-023 G3).

use bus::BotRuntime;

use crate::usage::limits;

use super::*;

impl Supervisor {
    /// Moves idle bots of a pool that hit a limit to `rate_limited`, and
    /// back to `ready` once it resets. A working bot is left to finish its
    /// turn: it ends on the limit error, goes idle, and is held next time.
    pub fn apply_pool_limits(&self) {
        let now = chrono::Utc::now();
        let Ok(bots) = self.inner.db.list_bots(None) else {
            return;
        };
        for (provider, runtime) in [
            (limits::CLAUDE, BotRuntime::ClaudeCode),
            (crate::usage::codex::PROVIDER, BotRuntime::CodexCli),
        ] {
            let until = match limits::limited_until(&self.inner.db, provider, now) {
                Ok(until) => until,
                Err(e) => {
                    tracing::debug!(provider, error = %e, "reading pool limits failed");
                    continue;
                }
            };
            for bot in bots
                .iter()
                .filter(|b| !b.is_linked() && b.runtime == runtime)
            {
                let state = self.state(&bot.id).0;
                match until {
                    Some(until) if state == BotState::Ready => {
                        let reason = format!(
                            "{provider} limit reached; resets {}",
                            until.format("%Y-%m-%d %H:%M UTC")
                        );
                        self.set_state(&bot.id, BotState::RateLimited, &reason);
                    }
                    None if state == BotState::RateLimited => {
                        self.set_state(&bot.id, BotState::Ready, "limit reset")
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::ProviderWindow;
    use crate::overrides::AutoCompactOverride;
    use crate::runtime::double::DoubleAdapter;

    /// A supervisor over three idle bots: two Claude Code, one Codex.
    fn supervisor(home: &std::path::Path) -> (Supervisor, Vec<String>) {
        let db = Db::open(&home.join("bus.sqlite")).expect("db");
        let project = db.create_project("p", "p").expect("project");
        let ids: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|name| {
                db.create_bot(&project.id, name, "", "", "", "/tmp/x", name, None)
                    .expect("bot")
                    .id
            })
            .collect();
        db.set_bot_runtime(&ids[2], BotRuntime::CodexCli)
            .expect("runtime");
        let cfg = Config {
            home: home.to_path_buf(),
            ..Config::default()
        };
        let secrets = Arc::new(Secrets::open(&home.join("secrets")).expect("secrets"));
        let sup = Supervisor::new(
            Arc::new(DoubleAdapter),
            cfg,
            db,
            Events::new(),
            secrets,
            AutoCompactOverride::default(),
        );
        for id in &ids {
            let mut handle = BotHandle::new(1024);
            handle.state = BotState::Ready;
            sup.lock_bots().insert(id.clone(), handle);
        }
        (sup, ids)
    }

    fn hit(sup: &Supervisor, provider: &str, until: chrono::DateTime<chrono::Utc>) {
        sup.inner
            .db
            .put_provider_window(&ProviderWindow {
                provider: provider.to_string(),
                window: "5h".to_string(),
                used_percent: Some(100.0),
                resets_at: Some(until),
                source: "observed".to_string(),
                capacity_estimate: None,
                limited_until: Some(until),
                updated_at: chrono::Utc::now(),
            })
            .expect("window");
    }

    #[test]
    fn a_claude_hit_holds_every_idle_claude_bot_until_the_reset() {
        let home = tempfile::tempdir().expect("tempdir");
        let (sup, ids) = supervisor(home.path());
        sup.set_state(&ids[1], BotState::Working, "busy");
        hit(
            &sup,
            "claude",
            chrono::Utc::now() + chrono::Duration::hours(1),
        );
        sup.apply_pool_limits();
        assert_eq!(sup.state(&ids[0]).0, BotState::RateLimited);
        assert!(sup.state(&ids[0]).1.starts_with("claude limit reached"));
        // A working bot finishes its turn first; the Codex pool is apart.
        assert_eq!(sup.state(&ids[1]).0, BotState::Working);
        assert_eq!(sup.state(&ids[2]).0, BotState::Ready);
        // Deliveries wait instead of failing.
        match sup.deliver(&ids[0], "hi") {
            Err(DeliverError::NotReady(reason)) => assert!(reason.contains("limit reached")),
            other => panic!("{other:?}"),
        }
        // Once the turn ends the bot is held too.
        sup.set_state(&ids[1], BotState::Ready, "turn complete");
        sup.apply_pool_limits();
        assert_eq!(sup.state(&ids[1]).0, BotState::RateLimited);
    }

    #[test]
    fn the_hold_lifts_at_the_reset() {
        let home = tempfile::tempdir().expect("tempdir");
        let (sup, ids) = supervisor(home.path());
        hit(
            &sup,
            "codex",
            chrono::Utc::now() + chrono::Duration::hours(1),
        );
        sup.apply_pool_limits();
        assert_eq!(sup.state(&ids[2]).0, BotState::RateLimited);
        assert_eq!(sup.state(&ids[0]).0, BotState::Ready);
        hit(
            &sup,
            "codex",
            chrono::Utc::now() - chrono::Duration::seconds(1),
        );
        sup.apply_pool_limits();
        assert_eq!(sup.state(&ids[2]).0, BotState::Ready);
    }
}
