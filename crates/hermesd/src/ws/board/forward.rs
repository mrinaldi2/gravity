//! An item's details on a board homed on a peer (B9 follow-up, ARCH-R28 c):
//! the owner's item drawer on imac or win-pc asks its own daemon, which
//! forwards `item_get`, `item_history` and `item_move_check` to the home as
//! `board_read` and answers from the connection's task once the home has.
//! Moves stay on the home, so the move check says so on every column.

use bus::contract::board::{self as c, board_request::Request, board_response::Response};
use serde_json::json;

use super::reads::{history_page, item_detail, move_check};
use super::{binary, Refusal};
use crate::actor::Actor;
use crate::app::AppState;
use crate::peer::board::{home_name, ids_from_home};
use crate::peer::{error_code, PeerError};
use crate::ws::Conn;

/// On the home: one of the forwarded reads, for an item of `project_id`. The
/// move check is the owner's, which the asking side marks as elsewhere.
pub(crate) fn peer_read(
    app: &AppState,
    project_id: &str,
    request: c::BoardRequest,
) -> anyhow::Result<c::BoardResponse> {
    let in_project = |id: &str| -> anyhow::Result<()> {
        match app.db.item_project(id)? {
            Some(p) if p == project_id => Ok(()),
            _ => Err(crate::peer::refuse("not_found", format!("no item {id}"))),
        }
    };
    let refusal = |r: Refusal| crate::peer::refuse(r.code, r.message);
    let response = match request.request {
        Some(Request::ItemGet(r)) => {
            in_project(&r.id)?;
            Response::Item(item_detail(app, &r.id).map_err(refusal)?)
        }
        Some(Request::ItemHistory(r)) => {
            in_project(&r.id)?;
            Response::History(history_page(app, &r).map_err(refusal)?)
        }
        Some(Request::ItemMoveCheck(r)) => {
            in_project(&r.id)?;
            Response::MoveCheck(move_check(app, &r.id, &Actor::User).map_err(refusal)?)
        }
        _ => anyhow::bail!("only item reads are forwarded"),
    };
    Ok(c::BoardResponse {
        response: Some(response),
    })
}

impl Conn {
    /// Sends a read about a mirrored item to the board's home, answering
    /// `req_id` when the home has.
    pub(super) fn forward_read(
        &self,
        req_id: u64,
        project_id: String,
        home: String,
        request: Request,
    ) {
        let app = self.app.clone();
        self.spawn_frame(req_id, "board_read", async move {
            let item_id = item_of(&request).to_string();
            let frame = json!({
                "type": "board_read", "project_id": project_id,
                "request": c::BoardRequest { request: Some(request) },
            });
            let name = home_name(&app, &home);
            match app.peers.request(&home, frame).await {
                Ok(mut value) => match app.db.project_link(&project_id, &home) {
                    Ok(Some(link)) => {
                        ids_from_home(&app, &link, &mut value["response"]);
                        decode(&value["response"], &name, &item_id)
                            .map(|response| binary::response(req_id, response))
                            .unwrap_or_else(|| {
                                binary::error(
                                    req_id,
                                    "internal",
                                    "unreadable answer from the board's home".into(),
                                )
                            })
                    }
                    _ => binary::error(
                        req_id,
                        "not_linked",
                        "the project is no longer linked".into(),
                    ),
                },
                Err(PeerError::Offline) => binary::error(
                    req_id,
                    "unavailable",
                    format!(
                        "{item_id} is on the board {name} holds, which can't be reached right now."
                    ),
                ),
                Err(e) => {
                    let e = anyhow::Error::from(e);
                    binary::error(req_id, error_code(&e, "internal"), e.to_string())
                }
            }
        });
    }
}

fn item_of(request: &Request) -> &str {
    match request {
        Request::ItemGet(r) => &r.id,
        Request::ItemHistory(r) => &r.id,
        Request::ItemMoveCheck(r) => &r.id,
        _ => "",
    }
}

/// The home's answer, with every move marked as the home's to make.
fn decode(value: &serde_json::Value, home: &str, id: &str) -> Option<c::BoardResponse> {
    let mut response: c::BoardResponse = serde_json::from_value(value.clone()).ok()?;
    if let Some(Response::MoveCheck(check)) = response.response.as_mut() {
        for column in &mut check.columns {
            column.unmet.push(c::Unmet {
                code: "board.elsewhere".into(),
                text: format!("The board lives on {home}. Move {id} from there."),
                fix: None,
            });
        }
    }
    Some(response)
}
