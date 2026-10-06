//! The overview's answer: this computer's part of each project, merged with
//! each linked peer's last good part (H-128 §2.3). Nothing here asks a peer.

use std::cmp::Ordering;
use std::collections::HashMap;

use bus::contract::board as c;
use bus::contract::home::{
    project_attention::Part, source::State, AttentionSummary, DoingBrief, Member, ProjectRow,
    ProjectsOverview, Source,
};
use bus::Peer;

use super::PeerEntry;
use crate::app::AppState;
use crate::attention::{self, cut, key, timestamp};
use crate::board::model::{ColumnCategory, ItemCard};
use crate::db::Db;

/// The most Doing cards a row lists.
const DOING_SHOWN: usize = 3;
/// The longest Doing card title a row carries.
const DOING_TITLE_MAX: usize = 80;

/// A linked peer and how its last request went.
struct Asked {
    peer: Peer,
    entry: PeerEntry,
}

/// The overview of the live projects, or of `project_ids` when given.
pub fn overview(app: &AppState, project_ids: &[String]) -> anyhow::Result<ProjectsOverview> {
    app.overview.watched_now();
    let me = app.db.daemon_id()?;
    let computer = app
        .db
        .board_read(crate::board::release::machines::this_computer)?;
    let asked: HashMap<String, Asked> = app
        .db
        .list_peers()?
        .into_iter()
        .filter(|p| p.revoked_at.is_none())
        .filter(|p| !app.db.links_through(&p.id).unwrap_or_default().is_empty())
        .map(|peer| {
            let entry = app.overview.entry(&peer.id);
            (peer.id.clone(), Asked { peer, entry })
        })
        .collect();
    let mut rows = Vec::new();
    for project in app.db.list_projects()? {
        if !project_ids.is_empty() && !project_ids.contains(&project.id) {
            continue;
        }
        rows.push(row(app, &project, &me, &computer, &asked)?);
    }
    rank(&mut rows);
    let mut total = AttentionSummary::default();
    for row in &rows {
        if let Some(summary) = &row.attention {
            attention::add(&mut total, summary);
        }
    }
    let mut sources: Vec<Source> = asked.values().map(source).collect();
    sources.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ProjectsOverview {
        as_of: Some(timestamp(chrono::Utc::now())),
        rows,
        sources,
        total: Some(total),
    })
}

fn source(asked: &Asked) -> Source {
    Source {
        daemon_id: asked.peer.daemon_id.clone().unwrap_or_default(),
        name: Db::display_peer_name(&asked.peer),
        state: asked.entry.state as i32,
        as_of: asked.entry.as_of.map(timestamp),
    }
}

fn row(
    app: &AppState,
    project: &bus::Project,
    me: &str,
    computer: &str,
    asked: &HashMap<String, Asked>,
) -> anyhow::Result<ProjectRow> {
    let db = &app.db;
    let here = super::part(app, &project.id)?;
    let mut members = vec![Member {
        daemon_id: me.to_string(),
        project_id: project.id.clone(),
        computer_name: computer.to_string(),
    }];
    let mut parts: Vec<&Part> = vec![&here];
    let mut stale = Vec::new();
    for link in db.project_links(&project.id)? {
        let Some(peer) = asked.get(&link.peer_id) else {
            continue;
        };
        let daemon_id = peer.peer.daemon_id.clone().unwrap_or_default();
        // A peer whose daemon id isn't known yet can't be merged by clients
        // (R2.3); it still counts here.
        if !daemon_id.is_empty() {
            members.push(Member {
                daemon_id: daemon_id.clone(),
                project_id: link.remote_project_id.clone(),
                computer_name: Db::display_peer_name(&peer.peer),
            });
        }
        if peer.entry.state != State::Ok {
            stale.push(daemon_id);
        }
        parts.extend(
            peer.entry
                .parts
                .iter()
                .filter(|p| p.project_id == project.id),
        );
    }
    let mut summary = AttentionSummary::default();
    for part in &parts {
        if let Some(local) = &part.local {
            attention::add(&mut summary, local);
        }
    }
    let home = parts.iter().find(|p| p.is_home);
    let bots = db.list_bots(Some(&project.id))?;
    let last_activity_at = parts
        .iter()
        .filter_map(|p| p.last_activity_at)
        .max_by_key(|t| (t.seconds, t.nanos));
    Ok(ProjectRow {
        project_id: project.id.clone(),
        name: Db::display_project_name(project),
        members,
        board_home: board_home(app, &project.id)?,
        current_release: home.and_then(|p| p.current_release.clone()),
        open_tasks: db.project_open_tasks(&project.id)?,
        doing: doing(app, &project.id)?,
        bots: u32::try_from(bots.len()).unwrap_or(u32::MAX),
        bots_working: parts.iter().map(|p| p.bots_working).sum(),
        attention: Some(summary),
        partial: !stale.is_empty(),
        stale_sources: stale,
        last_activity_at,
        pinned: parts.iter().any(|p| p.pinned),
        rank: 0,
        latest_summary: home.and_then(|p| p.latest_summary.clone()),
        legacy: false,
    })
}

/// The daemon id of the computer holding the project's board, or empty.
fn board_home(app: &AppState, project_id: &str) -> anyhow::Result<String> {
    if let Some(settings) = app.db.board_settings(project_id)? {
        return Ok(settings.home_daemon_id);
    }
    let Some(peer_id) = app.board_mirror.home_peer(project_id) else {
        return Ok(String::new());
    };
    Ok(app
        .db
        .get_peer(&peer_id)?
        .and_then(|p| p.daemon_id)
        .unwrap_or_default())
}

/// Up to three Doing cards, in board order: the board's here, or its
/// mirror off-home (B9).
fn doing(app: &AppState, project_id: &str) -> anyhow::Result<Vec<DoingBrief>> {
    let db = &app.db;
    let (doing_keys, cards): (Vec<String>, Vec<ItemCard>) =
        if db.board_settings(project_id)?.is_some() {
            let keys = db
                .board_columns(project_id)?
                .into_iter()
                .filter(|c| c.category == ColumnCategory::Doing)
                .map(|c| c.key)
                .collect();
            (keys, db.board_cards(project_id)?)
        } else if let Some(board) = app.board_mirror.get(project_id) {
            let keys = board
                .snapshot
                .columns
                .iter()
                .filter(|c| c.category() == c::ColumnCategory::Doing)
                .map(|c| c.key.clone())
                .collect();
            let cards = board
                .snapshot
                .cards
                .into_iter()
                .filter_map(|c| c.try_into().ok())
                .collect();
            (keys, cards)
        } else {
            return Ok(Vec::new());
        };
    cards
        .into_iter()
        .filter(|c| doing_keys.contains(&c.column_key))
        .take(DOING_SHOWN)
        .map(|card| {
            let assignee = card.assignee.unwrap_or_default();
            let name = match db.get_bot(&assignee)? {
                Some(bot) => Db::display_name(&bot),
                None => assignee.clone(),
            };
            Ok(DoingBrief {
                item_id: card.id,
                title: cut(&card.title, DOING_TITLE_MAX),
                assignee_name: name,
                assignee_bot_id: assignee,
            })
        })
        .collect()
}

/// Ranks rows (UX §5.4): pinned first, then score desc, oldest attention
/// first, most recent activity first, then name; `rank` numbers the result.
pub(super) fn rank(rows: &mut [ProjectRow]) {
    rows.sort_by(compare);
    for (n, row) in rows.iter_mut().enumerate() {
        row.rank = u32::try_from(n).unwrap_or(u32::MAX);
    }
}

fn compare(a: &ProjectRow, b: &ProjectRow) -> Ordering {
    let score = |r: &ProjectRow| r.attention.as_ref().map_or(0, |s| s.score);
    let oldest = |r: &ProjectRow| key(r.attention.as_ref().and_then(|s| s.oldest_at.as_ref()));
    // Newest activity first; none last.
    let active = |r: &ProjectRow| {
        r.last_activity_at
            .as_ref()
            .map_or((1, 0, 0), |t| (0, -t.seconds, -t.nanos))
    };
    b.pinned
        .cmp(&a.pinned)
        .then_with(|| score(b).cmp(&score(a)))
        .then_with(|| oldest(a).cmp(&oldest(b)))
        .then_with(|| active(a).cmp(&active(b)))
        .then_with(|| a.name.cmp(&b.name))
}
