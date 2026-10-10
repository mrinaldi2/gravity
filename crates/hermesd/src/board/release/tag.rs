//! The daemon's side of `hermesd release tag` (H-272; H-261 §6.3): DevOps
//! tags a release cut from main once the owner approved it. Main is already
//! at the commit, so nothing but the tag is pushed, and no release branch
//! exists.
//!
//! - `hermes/release_tag {release_id}`: the bot holds `release_main` and is
//!   the project's DevOps; the release was cut from main, the owner approved
//!   it, and its builds were made from the cut commit. Answers the commit,
//!   the tag name (`desktop-v<version>`) and the repo to check.
//! - `hermes/release_tagged {release_id, tag, commit}`: the daemon asks the
//!   repository itself that the tag is on the commit, then records it.

use std::sync::Arc;

use bus::PermissionExtra;
use chrono::Utc;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::bot_permissions::Stored;
use crate::decisions::{conflict, forbidden, invalid};
use crate::prs::repo;

use super::cut::{tag_name, version_of};
use super::gates::{approved, recorded_commit};
use super::model::{Release, ReleaseEvent};
use super::{git, load, Caller};

const TAG: &str = "hermes/release_tag";
const TAGGED: &str = "hermes/release_tagged";

/// Answers one of the tag gates from `bot_id`'s session, or `None`.
pub async fn serve(app: &Arc<AppState>, bot_id: &str, request: &Value) -> Option<Value> {
    let method = request.get("method").and_then(Value::as_str)?;
    if ![TAG, TAGGED].contains(&method) {
        return None;
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or(json!({}));
    let (app, bot_id, method) = (app.clone(), bot_id.to_string(), method.to_string());
    let answer = tokio::task::spawn_blocking(move || answer(&app, &bot_id, &method, &params))
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("the tag gate failed: {e}")));
    Some(match answer {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id,
                          "error": { "code": -32001, "message": e.to_string() } }),
    })
}

fn text<'a>(params: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid(format!("'{key}' is required")))
}

pub fn answer(
    app: &Arc<AppState>,
    bot_id: &str,
    method: &str,
    params: &Value,
) -> anyhow::Result<Value> {
    let bot = app
        .db
        .get_live_bot(bot_id)?
        .ok_or_else(|| forbidden("no such bot"))?;
    let release = gate(app, &bot, text(params, "release_id")?)?;
    let cut = release.cut.clone().expect("gate checked the cut");
    let tag = tag_name(&release);
    let repo = repo::of_project(app, &release.project_id, Some(&cut.repo))?;
    if method == TAG {
        return Ok(json!({
            "release_id": release.id, "name": release.name, "version": version_of(&release),
            "commit": cut.source_commit, "tag": tag, "repo": repo.name, "repo_url": repo.url,
        }));
    }
    if text(params, "commit")? != cut.source_commit || text(params, "tag")? != tag {
        return Err(forbidden(format!(
            "release {} is tagged {tag} at {}, nothing else",
            release.name, cut.source_commit
        )));
    }
    let peeled = format!("refs/tags/{tag}^{{}}");
    let refs = git::remote_refs(&repo.url, std::slice::from_ref(&peeled))?;
    let at = refs
        .iter()
        .find(|(n, _)| *n == peeled)
        .map(|(_, s)| s.as_str());
    if at != Some(cut.source_commit.as_str()) {
        return Err(conflict(format!(
            "{} has {tag} at {}, not {}; nothing recorded",
            repo.name,
            at.unwrap_or("nothing"),
            cut.source_commit
        )));
    }
    let event = ReleaseEvent {
        release_id: release.id.clone(),
        release_name: release.name.clone(),
        related_id: None,
        kind: "tagged".into(),
        actor: format!("bot:{}", bot.id),
        note: Some(format!("tagged {tag} on main")),
        detail: json!({ "commit": cut.source_commit, "tag": tag }),
        at: Utc::now(),
    };
    app.db.board_tx(|t| {
        t.set_release_tag(&release.id, &tag)?;
        t.record_release_event(&event, &release.project_id)
    })?;
    Ok(json!({ "recorded": "tagged", "tag": tag, "commit": cut.source_commit }))
}

/// Who may, for which release: `release_main`, DevOps, a release cut from
/// main, approved by the owner, and built from the cut commit.
fn gate(app: &Arc<AppState>, bot: &bus::Bot, release_id: &str) -> anyhow::Result<Release> {
    if app.cfg.scratch {
        return Err(forbidden("a scratch daemon tags nothing"));
    }
    if !Stored::load(&app.db, bot)?
        .extras
        .contains(&PermissionExtra::ReleaseMain)
    {
        return Err(forbidden(
            "this needs the release_main extra; the owner grants it in your permissions",
        ));
    }
    if !Caller::load(app, bot)?.roles.contains(&Role::Devops) {
        return Err(forbidden("only the project's DevOps tags a release"));
    }
    let release = app
        .db
        .board_read(|t| load(t, &bot.project_id, release_id))
        .map_err(|_| invalid(format!("no release {release_id} on this computer's board")))?;
    let Some(cut) = &release.cut else {
        return Err(forbidden(format!(
            "release {} wasn't cut from main; it lands with `hermesd release land`",
            release.name
        )));
    };
    if !approved(release.status) {
        return Err(forbidden(format!(
            "release {} is {}: only a release the owner approved is tagged",
            release.name,
            release.status.as_str()
        )));
    }
    if let Some(built) = recorded_commit(&release)? {
        if built != cut.source_commit {
            return Err(forbidden(format!(
                "release {}'s builds come from {built}, not its cut {}",
                release.name, cut.source_commit
            )));
        }
    }
    Ok(release)
}
