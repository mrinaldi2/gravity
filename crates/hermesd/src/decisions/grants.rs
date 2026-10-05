//! Permission extras a decision's option grants (H-117): a bot asks the
//! owner "grant DevOps install?" as a decision, and the owner's ruling is
//! the grant. When the owner's own ruling picks an option, its grants apply
//! at once. A ruling a bot relayed applies once the owner confirms it.
//! Grants only add: an extra already held stays, and none is taken away.

use std::sync::Arc;

use bus::{Bot, Decision, DecisionOption, PermissionExtra};
use serde_json::{json, Value};

use crate::app::AppState;

use super::invalid;

/// Checks each option's grants against the project's bots, storing a bot
/// named by name as its id.
pub fn checked(
    app: &AppState,
    project_id: &str,
    mut options: Vec<DecisionOption>,
) -> anyhow::Result<Vec<DecisionOption>> {
    for option in &mut options {
        for grant in &mut option.grants {
            if PermissionExtra::parse(grant.extra.trim()).is_none() {
                return Err(invalid(format!(
                    "option '{}' grants unknown extra '{}'",
                    option.key, grant.extra
                )));
            }
            grant.extra = grant.extra.trim().to_string();
            let bot = match app.db.get_live_bot(&grant.bot)? {
                Some(b) if b.project_id == project_id => Some(b),
                _ => app.db.get_bot_by_name(project_id, grant.bot.trim())?,
            };
            let bot = bot.ok_or_else(|| {
                invalid(format!(
                    "option '{}' grants to '{}', who isn't a bot of this project",
                    option.key, grant.bot
                ))
            })?;
            grant.bot = bot.id;
        }
    }
    Ok(options)
}

/// Adds `extras` to a bot of this daemon, restarting it when that changed
/// what it holds. Returns the extras it now holds.
pub fn add_extras(
    app: &AppState,
    bot: &Bot,
    extras: &[PermissionExtra],
) -> anyhow::Result<Vec<PermissionExtra>> {
    let mut held = app.db.bot_permission_extras(&bot.id)?;
    let before = held.clone();
    held.extend_from_slice(extras);
    held.sort();
    held.dedup();
    if held != before {
        app.db.set_bot_permission_extras(&bot.id, &held)?;
        if let Err(e) = app.supervisor.restart_bot(&bot.id) {
            tracing::warn!(bot_id = %bot.id, error = %e, "bot not restarted for its new extras");
        }
        app.events
            .push(crate::events::Push::BotUpdated { bot: bot.clone() });
    }
    Ok(held)
}

/// Applies the grants of the option the ruling picked, and says on the
/// decision what was applied. Linked bots get theirs on their own computer.
pub fn apply(app: &Arc<AppState>, decision: &Decision) {
    let Some(picked) = decision.ruling.as_ref().and_then(|r| r.option.as_deref()) else {
        return;
    };
    let Some(option) = decision.options.iter().find(|o| o.key == picked) else {
        return;
    };
    if option.grants.is_empty() {
        return;
    }
    let mut by_bot: Vec<(String, Vec<PermissionExtra>)> = Vec::new();
    for g in &option.grants {
        let Some(extra) = PermissionExtra::parse(&g.extra) else {
            continue;
        };
        match by_bot.iter_mut().find(|(b, _)| *b == g.bot) {
            Some((_, list)) => list.push(extra),
            None => by_bot.push((g.bot.clone(), vec![extra])),
        }
    }
    let mut lines = Vec::new();
    for (bot_id, extras) in by_bot {
        let names: Vec<&str> = extras.iter().map(|e| e.as_str()).collect();
        let bot = match app.db.get_live_bot(&bot_id) {
            Ok(Some(b)) => b,
            _ => {
                lines.push(format!("{bot_id}: not granted, the bot is gone"));
                continue;
            }
        };
        let name = crate::db::Db::display_name(&bot);
        if bot.is_linked() {
            let (app, bot, title) = (app.clone(), bot.clone(), decision.title.clone());
            let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
            lines.push(format!(
                "{name}: {} (sent to its computer)",
                names.join(", ")
            ));
            tokio::spawn(async move { grant_there(&app, &bot, &names, &title).await });
            continue;
        }
        match add_extras(app, &bot, &extras) {
            Ok(_) => lines.push(format!("{name}: {}", names.join(", "))),
            Err(e) => lines.push(format!("{name}: not granted ({e})")),
        }
    }
    let body = format!("Applied this ruling's grants. {}.", lines.join("; "));
    if let Err(e) =
        app.db
            .insert_decision_comment(&decision.id, bus::CommentAuthorKind::User, None, &body)
    {
        tracing::warn!(error = %e, "could not note the applied grants");
    }
}

/// Asks the computer a linked bot runs on to add its extras.
async fn grant_there(app: &AppState, bot: &Bot, extras: &[String], title: &str) {
    let (Some(peer), Some(remote)) = (&bot.peer_id, &bot.remote_bot_id) else {
        return;
    };
    let frame = json!({ "type": "grant_extras", "bot_id": remote, "extras": extras,
                        "decision": title });
    if let Err(e) = app.peers.request(peer, frame).await {
        tracing::warn!(bot_id = %bot.id, error = %e, "linked bot's grants not applied");
    }
}

/// On the computer a linked bot runs on: a peer asks to add its extras,
/// for a ruling the owner gave there. Only bots exposed to that peer.
pub fn serve_grant(app: &AppState, peer_id: &str, frame: &Value) -> anyhow::Result<Value> {
    let bot_id = frame["bot_id"].as_str().unwrap_or_default();
    if !app.db.is_exposed_to_peer(peer_id, bot_id)? {
        return Err(crate::peer::refuse(
            "forbidden",
            "that bot isn't linked with this computer",
        ));
    }
    let bot = app
        .db
        .get_live_bot(bot_id)?
        .filter(|b| !b.is_linked())
        .ok_or_else(|| crate::peer::refuse("not_found", "no such bot here"))?;
    let extras: Vec<PermissionExtra> = frame["extras"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().and_then(PermissionExtra::parse))
        .collect();
    tracing::info!(bot = %bot.name, peer_id, decision = %frame["decision"], ?extras,
                   "extras granted by a ruling on a linked computer");
    let held = add_extras(app, &bot, &extras)?;
    let held: Vec<&str> = held.iter().map(|e| e.as_str()).collect();
    Ok(json!({ "extras": held }))
}
