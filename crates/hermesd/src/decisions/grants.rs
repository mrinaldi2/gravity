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

/// The extras a linked computer's ruling may grant here (ARCH-R51 S1c).
const REMOTE_GRANTABLE: [PermissionExtra; 3] = [
    PermissionExtra::Install,
    PermissionExtra::DaemonRestart,
    PermissionExtra::Quiesce,
];

/// Whether another computer's ruling may grant `extra` on this one. The
/// ruling's computer checks it too, before sending (H-163).
pub fn remote_grantable(extra: &str) -> bool {
    PermissionExtra::parse(extra).is_some_and(|e| REMOTE_GRANTABLE.contains(&e))
}

/// The sha256 over the canonical JSON of an option's grants: what a view
/// shows as `grants_sha` and a ruling sends back (ARCH-R51 M2).
pub fn sha(option: &DecisionOption) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_string(&option.grants).unwrap_or_default();
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// Checks each option's grants against the project's bots, storing a bot
/// named by name as its id.
pub fn checked(
    app: &AppState,
    project_id: &str,
    mut options: Vec<DecisionOption>,
) -> anyhow::Result<Vec<DecisionOption>> {
    for option in &mut options {
        // Only views carry it, computed: never what a caller sent.
        option.grants_sha = None;
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
    // What a linked bot's grant is bound to until it lands (CE-020).
    let sha = sha(option);
    let ruling = crate::db::GrantRuling {
        decision_id: &decision.id,
        grants_sha: &sha,
        decided_at: decision
            .ruling
            .as_ref()
            .map_or_else(chrono::Utc::now, |r| r.answered_at),
    };
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
            // Kept until that computer applies or refuses it (H-163).
            let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
            lines.push(super::grants_peer::queue(app, &ruling, &bot, &names));
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
    // The owner here changed this bot's extras after the ruling: that is the
    // newer word, and a late grant must not undo it (CE-020 M2).
    let decided_at = frame["decided_at"]
        .as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok());
    if let (Some(decided), Some(changed)) = (decided_at, app.db.extras_changed_at(&bot.id)?) {
        if changed > decided {
            return Err(crate::peer::refuse(
                "conflict",
                "the owner changed its extras here after the ruling; set it here",
            ));
        }
    }
    // A linked computer grants only what installs need (ARCH-R51 S1c). The
    // rest is refused one by one, said in the answer, rather than turning
    // the whole grant away (H-163): the grantable extras still apply.
    let mut extras = Vec::new();
    let mut refused = Vec::new();
    for name in frame["extras"].as_array().into_iter().flatten() {
        let name = name.as_str().unwrap_or_default();
        match PermissionExtra::parse(name) {
            Some(e) if REMOTE_GRANTABLE.contains(&e) => extras.push(e),
            Some(_) => refused.push(json!({"extra": name,
                "why": "it can't be granted from another computer; set it here"})),
            None => refused.push(json!({"extra": name, "why": "this computer doesn't know it"})),
        }
    }
    if extras.is_empty() {
        let why = refused
            .first()
            .and_then(|r| r["why"].as_str())
            .unwrap_or("nothing to grant");
        return Err(crate::peer::refuse("forbidden", why.to_string()));
    }
    let held = add_extras(app, &bot, &extras)?;
    // On record where this computer's owner can read it (ARCH-R51 S1b).
    let names: Vec<&str> = extras.iter().map(|e| e.as_str()).collect();
    crate::owner_action::audit(
        app,
        &format!("grant:{}", bot.id),
        &format!("peer:{peer_id}"),
        "granted",
        json!({ "bot": bot.name, "extras": names, "decision": frame["decision"] }),
    );
    let held: Vec<&str> = held.iter().map(|e| e.as_str()).collect();
    Ok(json!({ "extras": held, "refused": refused }))
}
