//! Keeping the projects home live (H-128 §2.4, D4). What changes a row (a
//! bot's state, a task or message, a card, a decision, an owner action, a
//! permission prompt, a meeting) marks its project; marks are pushed as one
//! `projects_overview_changed` at most every 2 s, and sent to the linked
//! peers as `project_attention_changed` in their ids, so they ask again.
//! While a client is watching, peers are also asked every minute.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bus::BotState;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use super::{DEBOUNCE, REFRESH_EVERY};
use crate::app::AppState;
use crate::events::Push;

/// Starts the watcher, the debounced pushes and the minute's refresh.
pub fn spawn(app: Arc<AppState>) {
    let (marks, marked) = mpsc::unbounded_channel::<Mark>();
    tokio::spawn(debounce(app.clone(), marked));
    tokio::spawn(watch_pushes(app.clone(), marks.clone()));
    tokio::spawn(watch_board(app.clone(), marks));
    tokio::spawn(tick(app));
}

/// A project to push, or every project (a change that names none).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mark {
    Project(String),
    All,
}

async fn watch_pushes(app: Arc<AppState>, marks: mpsc::UnboundedSender<Mark>) {
    let mut pushes = app.events.subscribe_push();
    loop {
        let push = match pushes.recv().await {
            Ok(push) => push,
            Err(RecvError::Lagged(_)) => {
                let _ = marks.send(Mark::All);
                continue;
            }
            Err(RecvError::Closed) => break,
        };
        if let Push::BotState {
            bot_id, state, at, ..
        } = &push
        {
            let since = (*state == BotState::WaitingForUser).then(|| {
                chrono::DateTime::parse_from_rfc3339(at)
                    .map_or_else(|_| chrono::Utc::now(), |t| t.with_timezone(&chrono::Utc))
            });
            app.overview.set_waiting(bot_id, since);
        }
        if let Some(mark) = mark_of(&app, &push) {
            if marks.send(mark).is_err() {
                break;
            }
        }
    }
}

/// The project a push changes the row of, if it changes one.
fn mark_of(app: &AppState, push: &Push) -> Option<Mark> {
    let bot = |id: &str| {
        app.db
            .get_bot(id)
            .ok()
            .flatten()
            .map(|b| Mark::Project(b.project_id))
    };
    match push {
        Push::BotState { bot_id, .. } => bot(bot_id),
        Push::BotUpdated { bot } => Some(Mark::Project(bot.project_id.clone())),
        Push::ProjectUpdated { project } => Some(Mark::Project(project.id.clone())),
        Push::MessageNew { message } => app
            .db
            .conversation_project(&message.conversation_id)
            .ok()
            .flatten()
            .map(Mark::Project),
        Push::DecisionUpdate { decision } => Some(Mark::Project(decision.project_id.clone())),
        Push::DecisionDeleted { .. } => Some(Mark::All),
        Push::OwnerActionUpdate { action } => action["project_id"]
            .as_str()
            .map(|p| Mark::Project(p.to_string())),
        Push::PermissionRequest { request } => bot(&request.bot_id),
        Push::PermissionResolved { bot_id, .. } => bot(bot_id),
        Push::MeetingEvent { project_id, .. }
        | Push::OwnerThreadUpdated { project_id, .. }
        | Push::ProjectPinned { project_id, .. } => Some(Mark::Project(project_id.clone())),
        _ => None,
    }
}

/// Card moves and edits, from the board's feed.
async fn watch_board(app: Arc<AppState>, marks: mpsc::UnboundedSender<Mark>) {
    let mut feed = app.board.subscribe();
    loop {
        let mark = match feed.recv().await {
            Ok(change) => Mark::Project(change.project_id.clone()),
            Err(RecvError::Lagged(_)) => Mark::All,
            Err(RecvError::Closed) => break,
        };
        if marks.send(mark).is_err() {
            break;
        }
    }
}

/// Collects marks for 2 s after the first, then pushes them once.
async fn debounce(app: Arc<AppState>, mut marked: mpsc::UnboundedReceiver<Mark>) {
    while let Some(first) = marked.recv().await {
        let mut marks = vec![first];
        let window = tokio::time::sleep(DEBOUNCE);
        tokio::pin!(window);
        loop {
            tokio::select! {
                () = &mut window => break,
                mark = marked.recv() => match mark {
                    Some(mark) => marks.push(mark),
                    None => break,
                },
            }
        }
        flush(&app, &marks);
    }
}

/// The marked projects, pushed to clients and told to their peers.
fn flush(app: &AppState, marks: &[Mark]) {
    let projects: BTreeSet<String> = if marks.contains(&Mark::All) {
        app.db
            .list_projects()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.id)
            .collect()
    } else {
        marks
            .iter()
            .filter_map(|m| match m {
                Mark::Project(id) => Some(id.clone()),
                Mark::All => None,
            })
            .collect()
    };
    if projects.is_empty() {
        return;
    }
    // Each peer hears of its own linked projects, in its ids.
    let mut by_peer: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for project in &projects {
        for link in app.db.project_links(project).unwrap_or_default() {
            by_peer
                .entry(link.peer_id)
                .or_default()
                .push(link.remote_project_id);
        }
    }
    for (peer_id, project_ids) in by_peer {
        app.peers.notify(
            &peer_id,
            json!({ "type": "project_attention_changed", "project_ids": project_ids }),
        );
    }
    app.events.push(Push::ProjectsOverviewChanged {
        project_ids: projects.into_iter().collect(),
    });
}

/// Every minute while watched, each linked peer is asked again.
async fn tick(app: Arc<AppState>) {
    let mut every = tokio::time::interval(REFRESH_EVERY);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    every.tick().await;
    loop {
        every.tick().await;
        if !app.overview.is_watched() {
            continue;
        }
        for peer in app.db.list_peers().unwrap_or_default() {
            if peer.revoked_at.is_none() && app.peers.is_online(&peer.id) {
                super::refresh(app.clone(), peer.id);
            }
        }
    }
}
