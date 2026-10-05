//! Owner actions (H-117 R1): a bot proposes an exact command on a card or
//! decision, and only the owner runs it, from the app or the phone.
//!
//! - **Immutable.** A proposal's fields are hashed (sha256 over canonical
//!   JSON) and the row can't change after insert; the owner's client sends
//!   back the hash it showed, and a run of anything else is refused.
//! - **Single use.** `proposed → running` is one guarded update; a second
//!   tap finds it running. Proposals expire after 24 hours.
//! - **Only the owner runs it.** There is no run tool for bots. The
//!   WebSocket run needs the owner (or a device with `approve`) on a client
//!   that renders owner actions, and a local connection must not come from
//!   a bot's own processes (`ws/owner_actions.rs`).
//! - **What runs is what was shown.** The stored content runs as one
//!   argument from memory, never from a file (ARCH-R49 M2); pinned files
//!   are hashed again first.
//! - **Output** goes in full to a 0600 log; clients, the proposing bot and
//!   the card get a redacted tail. Every step is in an append-only audit,
//!   mirrored to `<home>/logs/owner-actions.log`.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::events::Push;
use model::{OwnerAction, Pinned, Proposal, Shell, State};

pub mod model;
pub mod redact;
pub mod run;
mod running;
mod tell;

pub use running::{pin_drift, start_run};
use tell::tell;
pub use tell::{follow, is_elsewhere};
pub mod validate;

/// How long a proposal waits for the owner.
pub const EXPIRY_HOURS: i64 = 24;

/// What a bot proposes.
#[derive(Default)]
pub struct ProposeRequest<'a> {
    pub item_id: Option<&'a str>,
    pub decision_id: Option<&'a str>,
    /// `here` (default) or a linked machine's name (R3).
    pub target: Option<&'a str>,
    pub shell: Option<&'a str>,
    pub cwd: &'a str,
    pub content: &'a str,
    pub pinned: &'a [String],
    pub reason: &'a str,
    pub timeout_s: Option<u32>,
}

/// Paths a bot can write: bot workspaces and worktrees, artifacts, and the
/// owner's trusted folders where worktrees live.
pub(crate) fn writable_roots(app: &AppState) -> Vec<PathBuf> {
    let mut roots = vec![app.cfg.home.join("projects")];
    let home = &app.cfg.user_home;
    for path in &app.cfg.trusted_paths {
        roots.push(match path.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(path),
        });
    }
    roots
}

pub(crate) fn sha256_of(path: &str) -> anyhow::Result<String> {
    crate::quiesce::file_sha256(std::path::Path::new(path))
        .map_err(|e| invalid(format!("can't pin {path}: {e}")))
}

/// What clients see: the action, and the linked computer it runs on by
/// name.
pub fn view(app: &AppState, a: &OwnerAction) -> Value {
    let mut v = a.to_json();
    if is_elsewhere(app, a) {
        v["target_name"] = json!(crate::peer::owner_actions::target_name(
            app,
            &a.proposal.target_machine
        ));
    }
    v
}

/// Tells this computer's clients, and the computer that offered it (R3).
fn changed(app: &AppState, a: &OwnerAction) {
    app.events.push(Push::OwnerActionUpdate {
        action: Box::new(view(app, a)),
    });
    if let Some(peer) = a.offered_by() {
        app.peers.notify(
            peer,
            json!({ "type": "owner_action_update", "action": a.wire() }),
        );
    }
}

/// Appends to the audit and its log mirror; a failed mirror write is only
/// logged.
pub fn audit(app: &AppState, id: &str, actor: &str, event: &str, detail: Value) {
    if let Err(e) = app.db.audit_owner_action(id, actor, event, &detail) {
        tracing::warn!(error = %e, "owner action audit failed");
    }
    let line = json!({ "at": Utc::now(), "action": id, "actor": actor, "event": event,
                       "detail": detail });
    let path = app.cfg.home.join("logs").join("owner-actions.log");
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes()));
    if let Err(e) = written {
        tracing::warn!(error = %e, "owner action audit log unavailable");
    }
}

/// A bot proposes an action (validated, hashed, stored as is). One for a
/// linked computer is offered there first, and stored here as the copy the
/// target acknowledged (R3).
pub async fn propose(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    req: &ProposeRequest<'_>,
) -> anyhow::Result<OwnerAction> {
    let proposal = prepare(app, bot, req)?;
    if proposal.target_machine != app.db.daemon_id()? {
        return crate::peer::owner_actions::offer(app, proposal).await;
    }
    store(app, proposal, "bot", writable_roots(app))
}

/// A checked proposal. For a linked computer the pinned files are named
/// only: the target hashes its own.
fn prepare(app: &AppState, bot: &bus::Bot, req: &ProposeRequest<'_>) -> anyhow::Result<Proposal> {
    let here = app.db.daemon_id()?;
    let target = match req.target.map(str::trim) {
        None | Some("" | "here") => here.clone(),
        Some(name) => crate::peer::owner_actions::target_daemon(app, name)?,
    };
    let shell = match req.shell {
        Some(s) => Shell::parse(s.trim()).ok_or_else(|| invalid(format!("unknown shell {s}")))?,
        None if target == here => Shell::here(),
        // This computer's default may not run there.
        None => return Err(invalid(
            "name the shell for that computer: powershell or cmd on Windows, zsh or bash on a Mac",
        )),
    };
    validate::content(req.content)?;
    validate::visible("the reason", req.reason)?;
    validate::visible("cwd", req.cwd)?;
    if req.reason.trim().is_empty() {
        return Err(invalid("say why the owner should run it (reason)"));
    }
    if shell == Shell::Cmd && req.content.contains('\n') {
        return Err(invalid("cmd runs one line; use powershell for a script"));
    }
    if target == here && !std::path::Path::new(req.cwd).is_dir() {
        return Err(invalid(format!(
            "cwd {} isn't a folder on this computer",
            req.cwd
        )));
    }
    for (kind, id) in [("item", req.item_id), ("decision", req.decision_id)] {
        let Some(id) = id else { continue };
        let project = match kind {
            "item" => app.db.item_project(id)?,
            _ => app.db.get_decision(id)?.map(|d| d.project_id),
        };
        if project.as_deref() != Some(bot.project_id.as_str()) {
            return Err(invalid(format!("no {kind} {id} in this project")));
        }
    }
    let pinned_files = req
        .pinned
        .iter()
        .map(|path| {
            Ok(Pinned {
                path: path.clone(),
                sha256: if target == here {
                    sha256_of(path)?
                } else {
                    String::new()
                },
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Proposal {
        project_id: bot.project_id.clone(),
        proposed_by: format!("bot:{}", bot.id),
        item_id: req.item_id.map(str::to_string),
        decision_id: req.decision_id.map(str::to_string),
        target_machine: target,
        shell,
        cwd: req.cwd.to_string(),
        content: req.content.to_string(),
        pinned_files,
        reason: req.reason.trim().to_string(),
        timeout_s: validate::timeout(req.timeout_s)?,
    })
}

/// A new, waiting action for `proposal`, flagged against `writable`.
pub fn new_action(
    id: String,
    proposal: Proposal,
    origin: &str,
    writable: &[PathBuf],
) -> OwnerAction {
    let pinned: Vec<String> = proposal
        .pinned_files
        .iter()
        .map(|p| p.path.clone())
        .collect();
    let now = Utc::now();
    OwnerAction {
        id,
        sha256: proposal.sha256(),
        flags: validate::unpinned_writable(&proposal.content, writable, &pinned),
        origin: origin.to_string(),
        state: State::Proposed,
        created_at: now,
        expires_at: now + Duration::hours(EXPIRY_HOURS),
        run_by: None,
        run_at: None,
        finished_at: None,
        exit_code: None,
        output_path: None,
        output_tail: None,
        reject_reason: None,
        local_project_id: None,
        proposal,
    }
}

/// Stores a proposal as a new action: the daemon's own templates (R4) come
/// here too.
pub fn store(
    app: &AppState,
    proposal: Proposal,
    origin: &str,
    writable: Vec<PathBuf>,
) -> anyhow::Result<OwnerAction> {
    insert(app, new_action(bus::new_id(), proposal, origin, &writable))
}

/// Records a new action as it is, audited and announced.
pub fn insert(app: &AppState, action: OwnerAction) -> anyhow::Result<OwnerAction> {
    app.db.insert_owner_action(&action)?;
    audit(
        app,
        &action.id,
        &action.proposal.proposed_by,
        "proposed",
        json!({ "sha256": action.sha256, "origin": action.origin }),
    );
    changed(app, &action);
    Ok(action)
}

/// An action of `project_id` (this computer's project id).
pub fn load(app: &AppState, project_id: Option<&str>, id: &str) -> anyhow::Result<OwnerAction> {
    let _ = app.db.expire_owner_actions(Utc::now());
    match app.db.get_owner_action(id)? {
        Some(a) if project_id.is_none_or(|p| a.project_here() == p) => Ok(a),
        _ => Err(not_found(format!("no owner action {id}"))),
    }
}

/// The proposing bot withdraws its own proposal, on the computer it targets
/// too.
pub async fn withdraw(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    id: &str,
) -> anyhow::Result<OwnerAction> {
    let a = load(app, Some(&bot.project_id), id)?;
    let by = a.proposal.proposed_by.clone();
    if by != format!("bot:{}", bot.id) {
        return Err(forbidden("only the bot that proposed it can withdraw it"));
    }
    if is_elsewhere(app, &a) {
        return crate::peer::owner_actions::close_there(app, &a, State::Withdrawn, None, &by).await;
    }
    close_as(app, &a, State::Withdrawn, None, &by)
}

/// The owner (stored as `actor`) rejects a proposal; the proposer is told
/// why.
pub async fn reject(
    app: &Arc<AppState>,
    actor: &str,
    id: &str,
    reason: Option<&str>,
) -> anyhow::Result<OwnerAction> {
    let a = load(app, None, id)?;
    if is_elsewhere(app, &a) {
        return crate::peer::owner_actions::close_there(app, &a, State::Rejected, reason, actor)
            .await;
    }
    close_as(app, &a, State::Rejected, reason, actor)
}

/// Closes a waiting action here, rejected or withdrawn by `actor`.
pub(crate) fn close_as(
    app: &AppState,
    a: &OwnerAction,
    to: State,
    reason: Option<&str>,
    actor: &str,
) -> anyhow::Result<OwnerAction> {
    if !app.db.close_owner_action(&a.id, to, reason, Utc::now())? {
        return Err(conflict(format!(
            "owner action {} is {}",
            a.id,
            a.state.as_str()
        )));
    }
    audit(app, &a.id, actor, to.as_str(), json!({ "reason": reason }));
    let closed = load(app, None, &a.id)?;
    changed(app, &closed);
    tell(app, &closed);
    Ok(closed)
}

#[cfg(test)]
mod tests;
