//! The owner's "Start the board on this computer" (H-037, ARCH-R18 M1). A
//! linked project gets its board here only when every linked computer says
//! it has none, asked with the peers' own `list_projects`: a click on a
//! second computer must not make a second home.

use bus::contract::board::board_response::Response;
use bus::contract::board::{self as c};
use bus::ProjectLink;
use serde_json::{json, Value};

use super::reads::{snapshot, Enable};
use super::{binary, refuse, Refusal, Reply};
use crate::app::AppState;
use crate::ws::Conn;

impl Conn {
    /// A board that already exists is returned as it is. An unlinked
    /// project's is enabled at once; a linked one's after asking each peer,
    /// off the connection's task, so the answer is sent from there.
    pub(super) fn board_enable(&self, req_id: u64, project_id: String) -> Reply {
        let db = &self.app.db;
        let links = db.project_links(&project_id)?;
        if db.board_settings(&project_id)?.is_some() || links.is_empty() {
            self.enable_board(&project_id, Enable::Owner)?;
            return Ok(Some(Response::Board(self.snapshot(&project_id)?)));
        }
        let app = self.app.clone();
        self.spawn_frame(req_id, "board:BoardEnable", async move {
            match enable_linked(&app, &project_id, &links).await {
                Ok(board) => binary::response(
                    req_id,
                    c::BoardResponse {
                        response: Some(Response::Board(board)),
                    },
                ),
                Err(r) => binary::error(req_id, r.code, r.message),
            }
        });
        Ok(None)
    }
}

async fn enable_linked(
    app: &AppState,
    project_id: &str,
    links: &[ProjectLink],
) -> Result<c::BoardSnapshot, Refusal> {
    for link in links {
        peer_has_no_board(app, link).await?;
    }
    let db = &app.db;
    if !db
        .get_project(project_id)?
        .is_some_and(|p| p.deleted_at.is_none())
    {
        return Err(refuse("not_found", format!("no project {project_id}")));
    }
    let mut feed = app.board.writer();
    db.ensure_board(project_id, &db.daemon_id()?, None)?;
    // Relayed to the linked peers, which start mirroring it (B9).
    feed.publish(crate::board::feed::Change {
        project_id,
        kind: crate::board::feed::ChangeKind::SettingsChanged,
        item_id: "",
        card: None,
        from_column: None,
    });
    drop(feed);
    snapshot(app, project_id)?
        .ok_or_else(|| refuse("internal", "the board was enabled but is missing"))
}

/// Refuses unless the peer answers that its side of the link has no board.
/// An offline peer, or one too old to say (no `has_board`), can't confirm.
async fn peer_has_no_board(app: &AppState, link: &ProjectLink) -> Result<(), Refusal> {
    let peer = app
        .db
        .get_peer(&link.peer_id)?
        .map_or_else(|| "a linked computer".to_string(), |p| p.name);
    let listed = app
        .peers
        .request(&link.peer_id, json!({ "type": "list_projects" }))
        .await
        .ok();
    let has_board = listed
        .as_ref()
        .and_then(|l| l.get("projects")?.as_array())
        .and_then(|projects| {
            projects
                .iter()
                .find(|p| p.get("id").and_then(Value::as_str) == Some(&link.remote_project_id))
        })
        .and_then(|p| p.get("has_board")?.as_bool());
    match has_board {
        Some(false) => Ok(()),
        Some(true) => Err(refuse(
            "conflict",
            format!("This project's board already lives on {peer}; open it there."),
        )),
        None => Err(refuse(
            "no_board",
            format!(
                "Can't confirm {peer} has no board for this project: connect it \
                 (or update it to 0.15.1) first."
            ),
        )),
    }
}
