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
        let Some(peer) = c.peer.filter(|p| p.ip().is_loopback()) else {
            // A device over the network: its credential is the owner's word.
            return Ok(());
        };
        match crate::holders::procs::tcp_client(peer) {
            Some(pid) => match bot_of(self, pid) {
                Some(bot) => Err(forbidden(format!(
                    "this connection comes from bot {bot}'s session; only the owner runs \
                     owner actions"
                ))),
                None => Ok(()),
            },
            None if c.via_ticket || c.device => Ok(()),
            None => Err(forbidden(
                "can't tell which process this connection comes from; run it from the app",
            )),
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
            .map(|a| a.to_json())
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
        self.send(
            json!({ "type": "owner_action", "req_id": req_id, "action": a.to_json(),
                          "audit": audit }),
        );
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
        let a = owner_action::start_run(&self.app, &self.actor().as_stored(), id, sha)?;
        self.send(json!({ "type": "owner_action", "req_id": req_id, "action": a.to_json() }));
        Ok(())
    }

    pub(super) fn owner_action_reject(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let id = Self::str_field(req, "id")?;
        let reason = req.get("reason").and_then(Value::as_str);
        let a = owner_action::reject(&self.app, &self.actor(), id, reason)?;
        self.send(json!({ "type": "owner_action", "req_id": req_id, "action": a.to_json() }));
        Ok(())
    }
}
