//! A ruling's grants for bots on a linked computer (H-163). The grant is
//! kept here until that computer applies it or refuses it: sent at once,
//! again when the link comes back, and on a sweep, so a computer offline or
//! restarting at the time of the ruling still gets it. Every outcome is said
//! on the decision, where the owner reads it: granted there, refused there
//! and why, or waiting for that computer.

use std::sync::Arc;
use std::time::Duration;

use bus::{Bot, CommentAuthorKind};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::db::{GrantEnd, GrantRuling, PeerGrant};
use crate::peer::PeerError;

/// How often grants still waiting are tried again, besides on link-up.
const SWEEP: Duration = Duration::from_secs(60);

/// How long a grant may wait for its computer (CE-020 M2): after that the
/// owner sets it there, on what that computer holds by then.
const EXPIRY_HOURS: i64 = 24;

/// Whether the ruling a grant came from still stands as it was: settled,
/// not superseded (reopened or replaced) and on the same option grants
/// (CE-020 M1). A withdrawn decision isn't settled.
fn ruling_holds(decision: Option<&bus::Decision>, grant: &PeerGrant) -> bool {
    let Some(d) = decision else {
        return false;
    };
    let picked = d.ruling.as_ref().and_then(|r| r.option.as_deref());
    let option = d.options.iter().find(|o| Some(o.key.as_str()) == picked);
    d.state == bus::DecisionState::Settled
        && d.superseded_by_id.is_none()
        && option.is_some_and(|o| super::grants::sha(o) == grant.grants_sha)
}

/// What it takes to grant what another computer can't: the owner sets it on
/// the bot's own computer.
fn set_it_on(there: &str, name: &str) -> String {
    format!("set it on {there} or from your phone, in the app there under {name}'s permissions")
}

/// Keeps a linked bot's grants and starts sending them; the line the
/// ruling's "Applied this ruling's grants" comment says for it. Extras the
/// bot's computer won't take from another one (ARCH-R51 S1c) aren't sent:
/// each is refused here, on the decision, with where to set it instead.
pub fn queue(
    app: &Arc<AppState>,
    ruling: &GrantRuling<'_>,
    bot: &Bot,
    extras: &[String],
) -> String {
    let decision_id = ruling.decision_id;
    let name = crate::db::Db::display_name(bot);
    let Some(peer_id) = bot.peer_id.clone() else {
        return format!("{name}: not granted, it isn't linked");
    };
    let there = computer(app, &peer_id);
    let (sent, local_only): (Vec<String>, Vec<String>) = extras
        .iter()
        .cloned()
        .partition(|e| super::grants::remote_grantable(app, e));
    for extra in &local_only {
        say(
            app,
            decision_id,
            &format!(
            "Not granted on {there}: {name}'s {extra} can't be granted from another computer; {}.",
            set_it_on(&there, &name)
        ),
        );
    }
    if sent.is_empty() {
        return format!("{name}: not granted here, see below");
    }
    if let Err(e) = app.db.insert_peer_grant(ruling, &bot.id, &peer_id, &sent) {
        return format!("{name}: not granted ({e})");
    }
    let app = app.clone();
    tokio::spawn(async move { deliver(&app, Some(&peer_id)).await });
    format!("{name}: {} (sending to {there})", sent.join(", "))
}

/// Sends every grant still waiting for `peer_id` (every peer's when None).
pub async fn deliver(app: &Arc<AppState>, peer_id: Option<&str>) {
    let pending = match app.db.pending_peer_grants(peer_id) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "grants for linked computers not read");
            return;
        }
    };
    for grant in pending {
        send(app, &grant).await;
    }
}

/// Tries grants still waiting every [`SWEEP`], for links that came back
/// without a link-up the hub saw, or answers lost to a restart.
pub fn spawn_sweep(app: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SWEEP).await;
            deliver(&app, None).await;
        }
    });
}

fn computer(app: &AppState, peer_id: &str) -> String {
    app.db
        .get_peer(peer_id)
        .ok()
        .flatten()
        .map(|p| p.name)
        .unwrap_or_else(|| "its computer".to_string())
}

fn say(app: &AppState, decision_id: &str, body: &str) {
    if let Err(e) =
        app.db
            .insert_decision_comment(decision_id, CommentAuthorKind::System, None, body)
    {
        tracing::warn!(error = %e, "could not note a linked grant's outcome");
    }
}

async fn send(app: &Arc<AppState>, grant: &PeerGrant) {
    let there = computer(app, &grant.peer_id);
    let bot = app.db.get_bot(&grant.bot_id).ok().flatten();
    let Some((bot, remote)) = bot.and_then(|b| b.remote_bot_id.clone().map(|r| (b, r))) else {
        if let Ok(true) =
            app.db
                .end_peer_grant(&grant.id, GrantEnd::Refused, Some("the bot is gone"))
        {
            say(
                app,
                &grant.decision_id,
                &format!("Not granted on {there}: the bot is no longer linked."),
            );
        }
        return;
    };
    let name = crate::db::Db::display_name(&bot);
    let decision = app.db.get_decision(&grant.decision_id).ok().flatten();
    if !ruling_holds(decision.as_ref(), grant) {
        if let Ok(true) =
            app.db
                .end_peer_grant(&grant.id, GrantEnd::Cancelled, Some("the ruling changed"))
        {
            say(
                app,
                &grant.decision_id,
                &format!(
                    "No longer granted on {there}: the ruling changed, so {name} doesn't get {}.",
                    grant.extras.join(", ")
                ),
            );
        }
        return;
    }
    if chrono::Utc::now() - grant.created_at > chrono::Duration::hours(EXPIRY_HOURS) {
        if let Ok(true) =
            app.db
                .end_peer_grant(&grant.id, GrantEnd::Expired, Some("not delivered in a day"))
        {
            say(
                app,
                &grant.decision_id,
                &format!(
                "Not sent: {there} was unreachable for a day, so {name}'s {} wasn't granted; {}.",
                grant.extras.join(", "),
                set_it_on(&there, &name)
            ),
            );
        }
        return;
    }
    let title = decision.map(|d| d.title).unwrap_or_default();
    let frame = json!({ "type": "grant_extras", "bot_id": remote, "extras": grant.extras,
                        "decision": title, "decided_at": grant.decided_at.to_rfc3339() });
    match app.peers.request(&grant.peer_id, frame).await {
        Ok(result) => applied(app, grant, &there, &name, &result),
        // Offline, restarting or no answer: try again, and say so once.
        Err(e) if transient(&e) => {
            if let Ok(1) = app.db.retry_peer_grant(&grant.id, &e.to_string()) {
                say(
                    app,
                    &grant.decision_id,
                    &format!(
                        "Waiting for {there} to grant {name} {}: {e}. It's sent again when \
                     {there} is back.",
                        grant.extras.join(", ")
                    ),
                );
            }
        }
        // The computer weighed it and said no: final.
        Err(e) => {
            let reason = e.to_string();
            if let Ok(true) = app
                .db
                .end_peer_grant(&grant.id, GrantEnd::Refused, Some(&reason))
            {
                for extra in &grant.extras {
                    say(
                        app,
                        &grant.decision_id,
                        &format!(
                            "Not granted on {there}: {name}'s {extra} ({reason}); {}.",
                            set_it_on(&there, &name)
                        ),
                    );
                }
            }
        }
    }
}

/// A failure worth trying again: the computer was away, restarting, or
/// didn't answer. Anything it answered is final.
fn transient(e: &PeerError) -> bool {
    match e {
        PeerError::Offline => true,
        PeerError::Rejected(reason) => {
            reason == crate::peer::LINK_CLOSED || reason == crate::peer::NO_ANSWER
        }
        PeerError::Refused { .. } => false,
    }
}

/// The computer answered: what it applied, and each extra it refused on its
/// own (a computer on this version applies the rest, H-163).
fn applied(app: &AppState, grant: &PeerGrant, there: &str, name: &str, result: &Value) {
    let refused: Vec<(&str, &str)> = result["refused"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            (
                r["extra"].as_str().unwrap_or("?"),
                r["why"].as_str().unwrap_or("refused"),
            )
        })
        .collect();
    let granted: Vec<&str> = grant
        .extras
        .iter()
        .map(String::as_str)
        .filter(|e| !refused.iter().any(|(r, _)| r == e))
        .collect();
    let error = (!refused.is_empty()).then(|| {
        let names: Vec<&str> = refused.iter().map(|(e, _)| *e).collect();
        format!("refused: {}", names.join(", "))
    });
    let Ok(true) = app
        .db
        .end_peer_grant(&grant.id, GrantEnd::Applied, error.as_deref())
    else {
        return;
    };
    if !granted.is_empty() {
        say(
            app,
            &grant.decision_id,
            &format!("Granted on {there}: {name} now has {}.", granted.join(", ")),
        );
    }
    for (extra, why) in refused {
        say(
            app,
            &grant.decision_id,
            &format!(
                "Not granted on {there}: {name}'s {extra} ({why}); {}.",
                set_it_on(there, name)
            ),
        );
    }
}
