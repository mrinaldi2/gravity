//! Requests a peer makes of this daemon. A peer can see which bots and
//! projects exist, link a bot or a project, deliver messages to the bots
//! linked to it, and manage bots in projects linked through it. Nothing else.

use std::sync::Arc;

use bus::{Peer, RemoteBot};
use serde_json::{json, Value};

use crate::app::AppState;

pub(super) fn handle(app: &Arc<AppState>, peer_id: &str, frame: &Value) -> anyhow::Result<Value> {
    let peer = app
        .db
        .get_peer(peer_id)?
        .filter(|p| p.revoked_at.is_none())
        .ok_or_else(|| anyhow::anyhow!("peer revoked"))?;
    match frame.get("type").and_then(Value::as_str).unwrap_or("") {
        "ping" => Ok(json!({})),
        "list_bots" => list_bots(app),
        "link" => link(app, &peer, frame),
        "chat" | "chat_step" | "chat_image" | "read_file" | "browser_activity" | "bot_commands" => {
            super::chat::serve(app, &peer, frame)
        }
        "term_attach" => super::term::serve_attach(app, &peer, frame),
        "restart_bot" | "clear_bot_session" => super::term::serve_session(app, &peer, frame),
        "browser_watch" => super::browser::serve_watch(app, &peer, frame),
        "term_detach" => super::term::serve_detach(app, &peer, frame),
        "list_projects" => super::links::serve_list(app, &peer),
        "link_project" => super::links::serve_link(app, &peer, frame),
        "unlink_project" => super::links::serve_unlink(app, &peer, frame),
        "board_call" => super::board_home::serve_call(app, &peer, frame),
        "board_snapshot" => super::board_home::serve_snapshot(app, &peer, frame),
        "board_read" => super::board_home::serve_read(app, &peer, frame),
        "pr_read" => super::pr_relay::serve_read(app, &peer, frame),
        "pr_owner" => super::pr_owner::serve(app, &peer, frame),
        "owner_action_offer" => super::owner_actions::serve_offer(app, &peer, frame),
        "owner_action_run" => super::owner_actions::serve_run(app, &peer, frame),
        "owner_action_close" => super::owner_actions::serve_close(app, &peer, frame),
        "grant_extras" => crate::decisions::grants::serve_grant(app, &peer.id, frame),
        "metrics_get" => super::board_home::serve_metrics(app, &peer, frame),
        // Deprecated: answered for peers on 0.17.2 or earlier; removed by H-185.
        "dashboard_needs_you" => super::board_home::serve_needs_you(app, &peer, frame),
        "project_attention" => crate::overview::serve(app, &peer, frame),
        "project_pin" => crate::overview::serve_pin(app, &peer, frame),
        "owner_threads" | "owner_thread_get" | "owner_thread_read" => {
            crate::owner_threads::serve(app, &peer, frame)
        }
        "create_bot" => super::remote_bots::serve_create(app, &peer, frame),
        crate::prs::check_remote::RUN => crate::prs::check_remote::serve_run(app, &peer, frame),
        crate::cleanup::remote::REQUEST => crate::cleanup::remote::serve_request(app, &peer, frame),
        "update_bot" => super::remote_bots::serve_update(app, &peer, frame),
        "delete_bot" => super::remote_bots::serve_delete(app, &peer, frame),
        "message" => {
            let message = serde_json::from_value(frame["message"].clone())?;
            let received = super::receive::receive(app, &peer, message)?;
            Ok(serde_json::to_value(received)?)
        }
        #[cfg(test)]
        "test_panic" => panic!("a test peer handler panicked"),
        other => anyhow::bail!("unknown peer request '{other}'"),
    }
}

/// Test events applied, to show the event feed outlives one that panicked.
#[cfg(test)]
pub(super) static TEST_EVENTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// News a peer sends without asking anything back.
pub(super) fn event(app: &Arc<AppState>, peer_id: &str, frame: &Value) {
    let Ok(Some(peer)) = app.db.get_peer(peer_id) else {
        return;
    };
    if peer.revoked_at.is_some() {
        return;
    }
    match frame["type"].as_str().unwrap_or("") {
        "term_frames" => super::term::receive_frames(app, peer_id, frame),
        "browser_feed" => super::browser::receive(app, peer_id, frame),
        "browser_unwatch" => super::browser::serve_unwatch(app, &peer, frame),
        "browser_input" => super::browser::serve_input(app, &peer, frame),
        "term_input" | "term_resize" | "term_detach" => super::term::serve_event(app, &peer, frame),
        "chat_turns" => super::chat::receive_turns(app, peer_id, frame),
        "project_roster" => super::mirror::receive_roster(app, peer_id, frame),
        "project_links" => super::mirror::receive_links(app, peer_id, frame),
        "board_event" => super::board::receive_event(app, peer_id, frame),
        "pr_event" => super::pr_relay::receive_event(app, peer_id, frame),
        crate::machine_tools::FRAME => crate::machine_tools::receive(app, peer_id, frame),
        crate::prs::check_remote::RESULT => {
            crate::prs::check_remote::receive_result(app, peer_id, frame)
        }
        crate::cleanup::remote::RESULT => {
            crate::cleanup::remote::receive_result(app, peer_id, frame)
        }
        "owner_action_update" => super::owner_actions::receive_update(app, peer_id, frame),
        "owner_action_output" => super::owner_actions::receive_output(app, peer_id, frame),
        "project_attention_changed" => crate::overview::receive_changed(app, peer_id),
        "owner_thread_updated" => crate::owner_threads::receive_updated(app, peer_id, frame),
        #[cfg(test)]
        "test_panic" => panic!("a test peer event panicked"),
        #[cfg(test)]
        "test_count" => {
            TEST_EVENTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        _ => {}
    }
}

/// The bots a peer could link: every live bot that runs here.
fn list_bots(app: &Arc<AppState>) -> anyhow::Result<Value> {
    let projects: std::collections::HashMap<String, String> = app
        .db
        .list_projects()?
        .into_iter()
        .map(|p| (p.id.clone(), crate::db::Db::display_project_name(&p)))
        .collect();
    let bots: Vec<RemoteBot> = app
        .db
        .list_bots(None)?
        .into_iter()
        .filter(|bot| !bot.is_linked())
        .map(|bot| {
            let project = projects.get(&bot.project_id).cloned().unwrap_or_default();
            remote_view(&bot, project)
        })
        .collect();
    Ok(json!({ "bots": bots }))
}

/// Exposes one local bot to the peer, which is about to stand it in.
fn link(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let bot_id = frame
        .get("bot_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("'bot_id' is required"))?;
    let bot = app
        .db
        .get_live_bot(bot_id)?
        .filter(|b| !b.is_linked())
        .ok_or_else(|| anyhow::anyhow!("no bot with id {bot_id} runs on this daemon"))?;
    app.db.expose_bot_to_peer(&peer.id, &bot.id)?;
    tracing::info!(peer = %peer.name, bot = %bot.name, "bot linked to peer");
    Ok(json!({ "bot": remote_view(&bot, String::new()) }))
}

pub(super) fn remote_view(bot: &bus::Bot, project: String) -> RemoteBot {
    RemoteBot {
        id: bot.id.clone(),
        name: bot.name.clone(),
        description: bot.description.clone(),
        avatar: bot.avatar.clone(),
        runtime: bot.runtime,
        project,
        temporary: bot.temporary,
    }
}
