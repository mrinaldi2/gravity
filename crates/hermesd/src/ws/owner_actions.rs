//! Owner actions over the WebSocket (H-117 R1, R2): the owner's clients
//! list them, read one, run it or reject it. Built like the T4 terminal
//! card: only a client that says it renders owner actions gets them, and
//! only the owner's approve grant runs one.
//!
//! A run is also refused when the connection comes from a bot's own
//! processes: a local connection is traced to its process, and a process
//! under a bot session is refused even with an owner credential. When the
//! process can't be told (Windows, no lsof), only the app's one-time ticket
//! or a device credential may run, never the owner token file a bot could
//! read.

use std::net::SocketAddr;

use serde_json::{json, Value};

use bus::Capability;

use crate::decisions::forbidden;
use crate::events::Push;
use crate::owner_action;

use super::Conn;

/// The feature a client sends at hello to receive owner actions.
pub const FEATURE: &str = "owner_actions";

/// The feature a client sends when it shows what a decision's option
/// grants: only such a client may rule on one that grants (H-117).
pub const GRANTS_FEATURE: &str = "decision_grants";

/// How one connection may see and run owner actions.
#[derive(Debug, Clone, Copy)]
pub(super) struct Client {
    /// It said it renders owner actions.
    renders: bool,
    approve: bool,
    /// It holds a device credential, not the owner token.
    device: bool,
    /// It holds the app's one-time ticket (H-044 T4).
    via_ticket: bool,
    peer: Option<SocketAddr>,
    /// It shows what a decision's option grants.
    pub(super) shows_grants: bool,
}

impl Client {
    pub(super) fn new(
        features: &[String],
        caps: &[Capability],
        device: bool,
        via_ticket: bool,
        peer: Option<SocketAddr>,
    ) -> Self {
        Self {
            renders: features.iter().any(|f| f == FEATURE),
            approve: caps.contains(&Capability::Approve),
            device,
            via_ticket,
            peer,
            shows_grants: features.iter().any(|f| f == GRANTS_FEATURE),
        }
    }

    /// Whether this client gets `push`: owner action pushes only reach a
    /// client that renders them.
    pub(super) fn sees(&self, push: &Push) -> bool {
        self.renders
            || !matches!(
                push,
                Push::OwnerActionUpdate { .. } | Push::OwnerActionOutput { .. }
            )
    }
}

/// The bot whose session `pid` runs under, if any.
fn bot_of(conn: &Conn, pid: u32) -> Option<String> {
    use crate::bus_auth::session::{session_of, ProcessTable};
    let table = crate::bus_auth::os::OsProcessTable;
    let info = table.info(pid)?;
    let accepted = crate::bus_auth::os::now().unwrap_or(u64::MAX);
    let roots = conn.app.supervisor.session_roots();
    if let Some((_, bot)) = session_of(&roots, &table, info, accepted) {
        return Some(bot);
    }
    crate::holders::ledger::session_processes(None)
        .into_iter()
        .find(|e| e.pid == pid)
        .map(|e| e.bot_id)
}

impl Conn {
    /// Refused unless this connection is the owner's own client.
    fn owner_runs(&self) -> anyhow::Result<()> {
        let c = self.owner;
        if !c.renders {
            return Err(forbidden(
                "this client doesn't show owner actions, so it can't run one",
            ));
        }
        if !c.approve {
            return Err(forbidden("running an owner action needs the approve grant"));
        }
        // The owner token is the one credential a bot of the same user can
        // read, so it never runs or rejects one, from anywhere: only the
        // app's one-time ticket or a paired device does (ARCH-R51 M1).
        if !(c.device || c.via_ticket) {
            return Err(forbidden(
                "owner actions run only from the app or a paired device, not with the \
                 owner token",
            ));
        }
        // On top, a local connection from a bot's own session is refused.
        let bot = c
            .peer
            .filter(|p| p.ip().is_loopback())
            .and_then(crate::holders::procs::tcp_client)
            .and_then(|pid| bot_of(self, pid));
        match bot {
            Some(bot) => Err(forbidden(format!(
                "this connection comes from bot {bot}'s session; only the owner runs \
                 owner actions"
            ))),
            None => Ok(()),
        }
    }

    pub(super) fn owner_action_list(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project = req.get("project_id").and_then(Value::as_str);
        let _ = self.app.db.expire_owner_actions(chrono::Utc::now());
        let actions: Vec<Value> = self
            .app
            .db
            .list_owner_actions(project, 100)?
            .iter()
            .map(|a| owner_action::view(&self.app, a))
            .collect();
        self.send(json!({ "type": "owner_actions", "req_id": req_id, "actions": actions }));
        Ok(())
    }

    pub(super) fn owner_action_get(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let id = Self::str_field(req, "id")?;
        let a = owner_action::load(&self.app, None, id)?;
        owner_action::audit(
            &self.app,
            id,
            &self.actor().as_stored(),
            "viewed",
            json!({}),
        );
        let audit: Vec<Value> = self
            .app
            .db
            .owner_action_audit(id)?
            .into_iter()
            .map(|(at, actor, event)| json!({ "at": at, "actor": actor, "event": event }))
            .collect();
        self.send(json!({ "type": "owner_action", "req_id": req_id,
                    "action": owner_action::view(&self.app, &a),
                          "audit": audit }));
        Ok(())
    }

    /// `{id, sha256}`: the hash the client showed the owner.
    pub(super) fn owner_action_run(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let id = Self::str_field(req, "id")?;
        let sha = Self::str_field(req, "sha256")?;
        if let Err(e) = self.owner_runs() {
            owner_action::audit(
                &self.app,
                id,
                &self.actor().as_stored(),
                "refused",
                json!({ "why": e.to_string() }),
            );
            return Err(e);
        }
        let by = self.actor().as_stored();
        let a = owner_action::load(&self.app, None, id)?;
        if !owner_action::is_elsewhere(&self.app, &a) {
            let running = owner_action::start_run(&self.app, &by, id, sha)?;
            self.send(json!({ "type": "owner_action", "req_id": req_id,
                              "action": owner_action::view(&self.app, &running) }));
            return Ok(());
        }
        // It runs on a linked computer: forwarded now, or refused (R3).
        let (app, sha) = (self.app.clone(), sha.to_string());
        self.answer_action(req_id, async move {
            crate::peer::owner_actions::run_there(&app, &a, &sha, &by).await
        });
        Ok(())
    }

    /// Answers `req_id` with the action `work` ends with, off this task.
    fn answer_action(
        &self,
        req_id: &Value,
        work: impl std::future::Future<Output = anyhow::Result<owner_action::model::OwnerAction>>
            + Send
            + 'static,
    ) {
        let app = self.app.clone();
        self.spawn_reply(req_id, move |req_id| async move {
            match work.await {
                Ok(a) => json!({ "type": "owner_action", "req_id": req_id,
                                 "action": owner_action::view(&app, &a) }),
                Err(e) => {
                    let code = crate::decisions::error_code(&e)
                        .unwrap_or_else(|| crate::peer::error_code(&e, "internal"));
                    json!({ "type": "error", "req_id": req_id, "code": code,
                            "message": e.to_string() })
                }
            }
        });
    }

    pub(super) fn owner_action_reject(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let id = Self::str_field(req, "id")?;
        let reason = req.get("reason").and_then(Value::as_str);
        // The same gate as a run (ARCH-R51 M1).
        self.owner_runs()?;
        let (app, by) = (self.app.clone(), self.actor().as_stored());
        let (id, reason) = (id.to_string(), reason.map(str::to_string));
        self.answer_action(req_id, async move {
            owner_action::reject(&app, &by, &id, reason.as_deref()).await
        });
        Ok(())
    }
}
