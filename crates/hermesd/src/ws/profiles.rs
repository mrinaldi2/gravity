//! Permission profiles (H-031): the owner sets a project's profile and each
//! bot's extras. Both apply at a bot's next start, so the affected bots are
//! restarted at once; the app says so before the owner confirms.

use bus::{PermissionExtra, PermissionProfile};
use serde_json::{json, Value};

use super::Conn;

impl Conn {
    /// `bot_grants {bot_id}`: the extras linked computers granted this bot
    /// by a ruling there, newest first (ARCH-R51 S1b):
    /// `{type: "bot_grants", grants: [{at, from, extras, decision}]}`.
    pub(super) fn bot_grants(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let bot_id = Self::str_field(req, "bot_id")?;
        let grants: Vec<Value> = self
            .app
            .db
            .bot_grant_audit(bot_id)?
            .into_iter()
            .map(|(at, actor, detail)| {
                let from = actor
                    .strip_prefix("peer:")
                    .and_then(|id| self.app.db.get_peer(id).ok().flatten())
                    .map_or_else(|| "another computer".to_string(), |p| p.name);
                json!({ "at": at, "from": from, "extras": detail["extras"],
                        "decision": detail["decision"] })
            })
            .collect();
        self.send(json!({ "type": "bot_grants", "req_id": req_id, "grants": grants }));
        Ok(())
    }

    /// `set_project_permission_profile {project_id, profile}`.
    pub(super) fn set_project_permission_profile(
        &self,
        req_id: &Value,
        req: &Value,
    ) -> anyhow::Result<()> {
        let project_id = Self::str_field(req, "project_id")?;
        let Some(profile) = req
            .get("profile")
            .and_then(Value::as_str)
            .and_then(PermissionProfile::parse)
        else {
            self.reply_err(
                req_id,
                "invalid_request",
                "'profile' must be standard, trusted or full",
            );
            return Ok(());
        };
        // A stale client may still offer Full; it is off until it is safe.
        if profile == PermissionProfile::Full && !crate::bot_permissions::FULL_ENABLED {
            self.reply_err(
                req_id,
                "invalid_request",
                "the Full profile is disabled until bots run in a separate user account or VM; use Trusted",
            );
            return Ok(());
        }
        let Some(project) = self.app.db.get_live_project(project_id)? else {
            self.reply_err(req_id, "not_found", "project not found or already deleted");
            return Ok(());
        };
        if self.app.db.project_permission_profile(&project.id)? != profile {
            self.app
                .db
                .set_project_permission_profile(&project.id, profile)?;
            for bot in self
                .app
                .db
                .list_bots(Some(&project.id))?
                .into_iter()
                .filter(|b| !b.is_linked())
            {
                if let Err(error) = self.app.supervisor.restart_bot(&bot.id) {
                    tracing::warn!(bot_id = %bot.id, %error, "bot not restarted for its new permission profile");
                }
            }
            tracing::info!(project = %project.name, profile = profile.as_str(), "permission profile changed");
        }
        self.app.events.push(crate::events::Push::ProjectUpdated {
            project: project.clone(),
        });
        self.send(json!({
            "type": "project", "req_id": req_id,
            "project": super::project_view(&self.app, &project)
        }));
        Ok(())
    }

    /// `set_bot_permission_extras {bot_id, extras: [..]}`: replaces the set.
    pub(super) fn set_bot_permission_extras(
        &self,
        req_id: &Value,
        req: &Value,
    ) -> anyhow::Result<()> {
        let bot_id = Self::str_field(req, "bot_id")?;
        let Some(list) = req.get("extras").and_then(Value::as_array) else {
            self.reply_err(req_id, "invalid_request", "'extras' must be a list");
            return Ok(());
        };
        let mut extras = Vec::with_capacity(list.len());
        for value in list {
            match value.as_str().and_then(PermissionExtra::parse) {
                Some(extra) => extras.push(extra),
                None => {
                    self.reply_err(req_id, "invalid_request", &format!("unknown extra {value}"));
                    return Ok(());
                }
            }
        }
        let Some(bot) = self.app.db.get_live_bot(bot_id)? else {
            self.reply_err(req_id, "not_found", "bot not found");
            return Ok(());
        };
        if bot.is_linked() {
            self.reply_err(
                req_id,
                "invalid_request",
                "this bot runs on another computer; change it there",
            );
            return Ok(());
        }
        extras.sort();
        extras.dedup();
        if self.app.db.bot_permission_extras(&bot.id)? != extras {
            // Persist first: a refused write must leave the bot running as it
            // was, not restart it for extras it was never given (H-039).
            if let Err(error) = self.app.db.set_bot_permission_extras(&bot.id, &extras) {
                tracing::warn!(bot_id = %bot.id, error = %format!("{error:#}"), "bot extras not saved");
                self.reply_err(
                    req_id,
                    "internal",
                    &format!("the extras could not be saved: {error:#}"),
                );
                return Ok(());
            }
            if let Err(error) = self.app.supervisor.restart_bot(&bot.id) {
                tracing::warn!(bot_id = %bot.id, %error, "bot not restarted for its new extras");
            }
        }
        self.app
            .events
            .push(crate::events::Push::BotUpdated { bot: bot.clone() });
        self.send(json!({ "type": "bot", "req_id": req_id, "bot": self.bot_json(&bot) }));
        Ok(())
    }
}
