//! The board's read requests, each from one consistent snapshot.

use bus::contract::board as c;
use bus::Capability;

use super::{not_found, refuse, Refusal};
use crate::board::contract::MapError;
use crate::board::model as m;
use crate::board::moves;
use crate::ws::Conn;

/// History events per page when the client names no limit, and the most it
/// may ask for.
const HISTORY_PAGE: u32 = 100;
const HISTORY_MAX: u32 = 500;

impl Conn {
    /// The whole board, current with the project's push `seq`. The number is
    /// read first: a change landing in between is in the snapshot and comes
    /// again as a push, which the client applies harmlessly.
    pub(super) fn snapshot(&self, project_id: &str) -> Result<c::BoardSnapshot, Refusal> {
        let seq = self.app.board.seq(project_id);
        let db = &self.app.db;
        let snapshot = match db.board_read(|t| t.snapshot(project_id))? {
            Some(snapshot) => snapshot,
            None => {
                self.enable_board(project_id)?;
                db.board_read(|t| t.snapshot(project_id))?
                    .ok_or_else(|| refuse("internal", "the board was enabled but is missing"))?
            }
        };
        Ok(c::BoardSnapshot {
            settings: Some(snapshot.settings.into()),
            columns: snapshot.columns.into_iter().map(Into::into).collect(),
            cards: snapshot.cards.into_iter().map(Into::into).collect(),
            roles: snapshot.roles.into_iter().map(Into::into).collect(),
            seq,
        })
    }

    /// A project's first `board_get` enables its board (B2 defaults), on
    /// this daemon as its home. Enabling is a change, so it needs `control`.
    fn enable_board(&self, project_id: &str) -> Result<(), Refusal> {
        let db = &self.app.db;
        if !db
            .get_project(project_id)?
            .is_some_and(|p| p.deleted_at.is_none())
        {
            return Err(refuse("not_found", format!("no project {project_id}")));
        }
        if !self.caps.contains(&Capability::Control) {
            return Err(refuse(
                "no_board",
                "This project has no board yet; enabling it needs the control grant.",
            ));
        }
        db.ensure_board(project_id, &db.daemon_id()?, None)?;
        Ok(())
    }

    pub(super) fn item_get(&self, id: &str) -> Result<c::ItemDetail, Refusal> {
        self.app
            .db
            .board_read(|t| {
                let Some(item) = t.item(id)? else {
                    return Ok(None);
                };
                let (history, next) = t.item_history(id, None, HISTORY_PAGE)?;
                Ok(Some(c::ItemDetail {
                    item: Some(item.into()),
                    links: t.item_links(id)?.into_iter().map(Into::into).collect(),
                    comments: t.item_comments(id)?.into_iter().map(Into::into).collect(),
                    history: history.into_iter().map(Into::into).collect(),
                    history_next: next,
                }))
            })?
            .ok_or_else(|| not_found(id))
    }

    pub(super) fn item_history(&self, r: &c::ItemHistory) -> Result<c::ItemHistoryPage, Refusal> {
        let limit = match r.limit {
            0 => HISTORY_PAGE,
            n => n.min(HISTORY_MAX),
        };
        let (events, next) = self
            .app
            .db
            .board_read(|t| {
                if t.item_project(&r.id)?.is_none() {
                    return Ok(None);
                }
                t.item_history(&r.id, r.after, limit).map(Some)
            })?
            .ok_or_else(|| not_found(&r.id))?;
        Ok(c::ItemHistoryPage {
            item_id: r.id.clone(),
            events: events.into_iter().map(Into::into).collect(),
            next,
        })
    }

    pub(super) fn item_query(&self, r: &c::ItemQuery) -> Result<c::ItemQueryResult, Refusal> {
        let bad = |e: MapError| refuse("invalid_request", e.to_string());
        let types = wire_list(&r.types, m::ItemType::from_wire).map_err(bad)?;
        let priorities = wire_list(&r.priorities, m::Priority::from_wire).map_err(bad)?;
        let platforms = wire_list(&r.platforms, m::Platform::from_wire).map_err(bad)?;
        let text = r.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let cards = self.app.db.board_read(|t| match text {
            Some(text) => t.search_cards(&r.project_id, text),
            None => t.cards(&r.project_id),
        });
        let cards = match (cards, text) {
            (Ok(cards), _) => cards,
            (Err(e), Some(_)) => return Err(refuse("invalid_request", format!("search: {e}"))),
            (Err(e), None) => return Err(e.into()),
        };
        let wanted = |card: &m::ItemCard| {
            (r.column_keys.is_empty() || r.column_keys.contains(&card.column_key))
                && r.assignee
                    .as_ref()
                    .is_none_or(|a| card.assignee.as_ref() == Some(a))
                && (types.is_empty() || types.contains(&card.item_type))
                && (priorities.is_empty() || priorities.contains(&card.priority))
                && (platforms.is_empty() || platforms.iter().any(|p| card.platforms.contains(p)))
                && r.blocked.is_none_or(|b| card.blocked == b)
        };
        Ok(c::ItemQueryResult {
            cards: cards.into_iter().filter(wanted).map(Into::into).collect(),
        })
    }

    pub(super) fn item_move_check(&self, id: &str) -> Result<c::MoveCheck, Refusal> {
        if self.app.db.board_read(|t| t.item_project(id))?.is_none() {
            return Err(not_found(id));
        }
        let columns = moves::item_move_check(&self.app.db, id, &self.actor())?;
        Ok(c::MoveCheck {
            item_id: id.to_string(),
            columns: columns
                .into_iter()
                .map(|(column, unmet)| c::ColumnCheck {
                    column_key: column.key,
                    unmet: unmet.into_iter().map(Into::into).collect(),
                })
                .collect(),
        })
    }
}

fn wire_list<T>(
    numbers: &[i32],
    from_wire: impl Fn(i32) -> Result<T, MapError>,
) -> Result<Vec<T>, MapError> {
    numbers.iter().map(|n| from_wire(*n)).collect()
}
