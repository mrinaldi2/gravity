//! Owner actions on the computer they run on (H-117 R3): a linked
//! computer offers one, and runs, rejects or withdraws only what it offered.
//! The offer is checked here as a bot's proposal would be, the pinned files
//! are hashed here, and the run goes through the same single-use, hashed
//! claim as a run from this computer's own owner.

use std::sync::Arc;

use bus::Peer;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::decisions::error_code;
use crate::owner_action::model::{OwnerAction, Pinned, Proposal, Shell, State};
use crate::owner_action::{
    self, audit, new_action, sha256_of, start_run, validate, writable_roots,
};
use crate::peer::refuse;

/// A decision-style error as a refusal the peer link carries with its code.
fn coded(e: anyhow::Error) -> anyhow::Error {
    match error_code(&e) {
        Some(code) => refuse(code, e.to_string()),
        None => e,
    }
}

fn text<'a>(frame: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    frame
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("'{key}' is required"))
}

/// `owner_action_offer {id, proposal}`: keeps this computer's own copy.
pub(in crate::peer) fn serve_offer(
    app: &Arc<AppState>,
    peer: &Peer,
    frame: &Value,
) -> anyhow::Result<Value> {
    let id = text(frame, "id")?;
    let mut proposal: Proposal = serde_json::from_value(frame["proposal"].clone())?;
    let me = app.db.daemon_id()?;
    if proposal.target_machine != me {
        return Err(refuse(
            "invalid_request",
            "that action is for another computer",
        ));
    }
    let link = app
        .db
        .project_link_by_remote(&peer.id, &proposal.project_id)?
        .ok_or_else(|| refuse("not_linked", "that project isn't linked with this computer"))?;
    check(&proposal).map_err(coded)?;
    for pin in &mut proposal.pinned_files {
        *pin = Pinned {
            sha256: sha256_of(&pin.path).map_err(coded)?,
            path: std::mem::take(&mut pin.path),
        };
    }
    if app.db.get_owner_action(id)?.is_some() {
        return Err(refuse(
            "conflict",
            format!("owner action {id} exists already"),
        ));
    }
    let origin = format!("peer:{}", peer.id);
    let action = OwnerAction {
        local_project_id: Some(link.project_id),
        ..new_action(id.to_string(), proposal, &origin, &writable_roots(app))
    };
    let stored = owner_action::insert(app, action)?;
    Ok(json!({ "action": stored.wire() }))
}

/// What a proposal from this computer's own bots must be too.
fn check(p: &Proposal) -> anyhow::Result<()> {
    use crate::decisions::invalid;
    validate::content(&p.content)?;
    validate::visible("the reason", &p.reason)?;
    validate::visible("cwd", &p.cwd)?;
    validate::timeout(Some(p.timeout_s))?;
    if p.reason.trim().is_empty() {
        return Err(invalid("say why the owner should run it (reason)"));
    }
    if p.shell == Shell::Cmd && p.content.contains('\n') {
        return Err(invalid("cmd runs one line; use powershell for a script"));
    }
    if cfg!(windows) != matches!(p.shell, Shell::Powershell | Shell::Cmd) {
        return Err(invalid(format!(
            "{} doesn't run on this computer",
            p.shell.as_str()
        )));
    }
    if !std::path::Path::new(&p.cwd).is_dir() {
        return Err(invalid(format!(
            "cwd {} isn't a folder on this computer",
            p.cwd
        )));
    }
    Ok(())
}

/// The action `frame.id`, when `peer` offered it; refused and audited
/// otherwise.
fn offered(app: &AppState, peer: &Peer, frame: &Value, what: &str) -> anyhow::Result<OwnerAction> {
    let id = text(frame, "id")?;
    match app.db.get_owner_action(id)? {
        Some(a) if a.offered_by() == Some(peer.id.as_str()) => Ok(a),
        _ => {
            audit(
                app,
                id,
                &format!("peer:{}", peer.name),
                "refused",
                json!({ "why": format!("{what} for an action this computer never offered") }),
            );
            Err(refuse(
                "not_found",
                format!("no owner action {id} offered by this computer"),
            ))
        }
    }
}

/// `owner_action_run {id, sha256, approved_by}`: the owner approved it on
/// the computer that offered it.
pub(in crate::peer) fn serve_run(
    app: &Arc<AppState>,
    peer: &Peer,
    frame: &Value,
) -> anyhow::Result<Value> {
    let a = offered(app, peer, frame, "a run")?;
    let sha = text(frame, "sha256")?;
    let by = format!(
        "peer:{}/{}",
        peer.name,
        frame["approved_by"].as_str().unwrap_or("owner")
    );
    let running = start_run(app, &by, &a.id, sha).map_err(coded)?;
    Ok(json!({ "action": running.wire() }))
}

/// `owner_action_close {id, state, reason, actor}`: rejected by the owner
/// or withdrawn by its bot on the computer that offered it.
pub(in crate::peer) fn serve_close(
    app: &Arc<AppState>,
    peer: &Peer,
    frame: &Value,
) -> anyhow::Result<Value> {
    let a = offered(app, peer, frame, "a close")?;
    let to: State = serde_json::from_value(frame["state"].clone())?;
    if !matches!(to, State::Rejected | State::Withdrawn) {
        return Err(refuse("invalid_request", "only reject or withdraw"));
    }
    let actor = format!(
        "peer:{}/{}",
        peer.name,
        frame["actor"].as_str().unwrap_or("owner")
    );
    let reason = frame["reason"].as_str();
    let closed = owner_action::close_as(app, &a, to, reason, &actor).map_err(coded)?;
    Ok(json!({ "action": closed.wire() }))
}
