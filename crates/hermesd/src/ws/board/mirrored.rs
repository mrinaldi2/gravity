//! A board whose home is a peer, over this daemon's WebSocket (B9, H-020
//! §1.3): the board and its cards come from the mirror, kept current by the
//! home's relay, so a client attached here watches it move. Item details and
//! every change stay on the home, as do rulings.

use bus::contract::board::{self as c, board_request::Request};

use super::reads::query_cards;
use super::{refuse, Refusal};
use crate::board::model as m;
use crate::peer::board::home_name;
use crate::ws::Conn;

impl Conn {
    /// The mirrored board, current with this daemon's push `seq` for it.
    /// Read-only here, so it can't be ruled on.
    pub(super) fn mirrored_snapshot(&self, project_id: &str) -> Result<c::BoardSnapshot, Refusal> {
        let seq = self.app.board.seq(project_id);
        let mirrored = self
            .app
            .board_mirror
            .get(project_id)
            .ok_or_else(|| refuse("no_board", "This project has no board yet."))?;
        Ok(c::BoardSnapshot {
            seq,
            can_rule: false,
            ..mirrored.snapshot
        })
    }

    /// An `ItemQuery` on a mirrored board, or `None` when the project's
    /// board isn't mirrored here.
    pub(super) fn mirrored_query(
        &self,
        r: &c::ItemQuery,
    ) -> Option<Result<c::ItemQueryResult, Refusal>> {
        let mirrored = self.app.board_mirror.get(&r.project_id)?;
        let text = r
            .text
            .as_deref()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty());
        let cards = mirrored
            .snapshot
            .cards
            .into_iter()
            .filter(|card| {
                text.as_ref().is_none_or(|t| {
                    card.title.to_lowercase().contains(t) || card.id.eq_ignore_ascii_case(t)
                })
            })
            .filter_map(|card| m::ItemCard::try_from(card).ok())
            .collect();
        Some(query_cards(r, cards))
    }

    /// Refuses a request about one item of a board mirrored here: its
    /// details, its history and every change live on the board's home.
    pub(super) fn on_home(&self, request: &Request) -> Result<(), Refusal> {
        let id = match request {
            Request::ItemGet(r) => &r.id,
            Request::ItemHistory(r) => &r.id,
            Request::ItemMoveCheck(r) => &r.id,
            Request::ItemMove(r) => &r.id,
            _ => return Ok(()),
        };
        let mirror = &self.app.board_mirror;
        let Some(home) = mirror
            .project_of(id)
            .and_then(|project| mirror.home_peer(&project))
        else {
            return Ok(());
        };
        if self.app.db.board_read(|t| t.item_project(id))?.is_some() {
            return Ok(());
        }
        Err(refuse(
            "no_board",
            format!(
                "{id} is on the board {} holds; open it there to see its details or change it.",
                home_name(&self.app, &home)
            ),
        ))
    }
}
