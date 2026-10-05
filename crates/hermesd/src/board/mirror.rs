//! Boards whose home is a peer (B9, H-020 §1.3). A linked project's board
//! lives on one daemon; the others keep its last snapshot here, in their own
//! ids, kept current by the home's `board_event` relay. They serve reads from
//! it, so it is also the read-only board while the home is unreachable. It
//! lives in memory: a restart refetches it when the home's link comes up.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use bus::contract::board as c;

/// A peer's board as this daemon last saw it.
#[derive(Debug, Clone)]
pub struct Mirrored {
    /// The peer holding the board.
    pub peer_id: String,
    /// In this daemon's ids. Its `seq` is meaningless here: callers stamp
    /// the local feed's.
    pub snapshot: c::BoardSnapshot,
}

/// Local project id → the board mirrored for it.
#[derive(Clone, Default)]
pub struct BoardMirror {
    boards: Arc<Mutex<HashMap<String, Mirrored>>>,
}

impl BoardMirror {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Mirrored>> {
        self.boards.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self, project_id: &str) -> Option<Mirrored> {
        self.lock().get(project_id).cloned()
    }

    /// The peer holding the project's board, when one is known.
    pub fn home_peer(&self, project_id: &str) -> Option<String> {
        self.lock().get(project_id).map(|m| m.peer_id.clone())
    }

    pub fn set(&self, project_id: &str, peer_id: &str, snapshot: c::BoardSnapshot) {
        self.lock().insert(
            project_id.to_string(),
            Mirrored {
                peer_id: peer_id.to_string(),
                snapshot,
            },
        );
    }

    /// Forgets the board, when `peer_id` says it no longer holds it.
    pub fn forget(&self, project_id: &str, peer_id: &str) {
        let mut boards = self.lock();
        if boards.get(project_id).is_some_and(|m| m.peer_id == peer_id) {
            boards.remove(project_id);
        }
    }

    /// The mirrored project an item belongs to.
    pub fn project_of(&self, item_id: &str) -> Option<String> {
        self.lock()
            .iter()
            .find(|(_, m)| m.snapshot.cards.iter().any(|card| card.id == item_id))
            .map(|(project, _)| project.clone())
    }

    /// Takes a pushed card unless the mirror already holds a newer version
    /// (a refetch can overtake the pushes behind it). False when nothing
    /// changed, or the project isn't mirrored.
    pub fn apply_card(&self, project_id: &str, card: c::ItemCard) -> bool {
        let mut boards = self.lock();
        let Some(board) = boards.get_mut(project_id) else {
            return false;
        };
        let cards = &mut board.snapshot.cards;
        match cards.iter_mut().find(|held| held.id == card.id) {
            Some(held) if held.version > card.version => false,
            Some(held) => {
                *held = card;
                true
            }
            None => {
                cards.push(card);
                true
            }
        }
    }

    /// Drops a removed item. False when the project isn't mirrored.
    pub fn remove_card(&self, project_id: &str, item_id: &str) -> bool {
        let mut boards = self.lock();
        let Some(board) = boards.get_mut(project_id) else {
            return false;
        };
        board.snapshot.cards.retain(|card| card.id != item_id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, version: u64, column: &str) -> c::ItemCard {
        c::ItemCard {
            id: id.into(),
            version,
            column_key: column.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_pushed_card_never_rolls_back_a_newer_one() {
        let mirror = BoardMirror::default();
        assert!(!mirror.apply_card("p", card("H-1", 1, "ready")));
        mirror.set("p", "peer", c::BoardSnapshot::default());
        assert!(mirror.apply_card("p", card("H-1", 3, "doing")));
        assert!(!mirror.apply_card("p", card("H-1", 2, "ready")));
        assert!(mirror.apply_card("p", card("H-1", 4, "review")));
        let held = mirror.get("p").expect("mirrored").snapshot.cards;
        assert_eq!(held, [card("H-1", 4, "review")]);
        assert_eq!(mirror.project_of("H-1").as_deref(), Some("p"));
        assert!(mirror.remove_card("p", "H-1"));
        assert_eq!(mirror.project_of("H-1"), None);
    }

    #[test]
    fn only_the_holding_peer_can_withdraw_a_board() {
        let mirror = BoardMirror::default();
        mirror.set("p", "mac", c::BoardSnapshot::default());
        mirror.forget("p", "imac");
        assert_eq!(mirror.home_peer("p").as_deref(), Some("mac"));
        mirror.forget("p", "mac");
        assert_eq!(mirror.home_peer("p"), None);
    }
}
