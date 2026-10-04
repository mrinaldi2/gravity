//! Board pushes (H-020 §1.5): every committed board change is published
//! here with the next number of its project's sequence, and the WS adapter
//! forwards it to the connections watching that project.
//!
//! A writer holds the feed (`writer()`) across its transaction and its
//! publish, so changes go out in the order they committed and a sequence
//! number always follows the change it names. A client reads `seq()` before
//! its snapshot: a change that lands in between is in the snapshot and
//! arrives again as a push, which is harmless because a push carries the
//! whole card.
//!
//! The sequence lives in memory and starts again at 0 when the daemon does.
//! Clients treat any number other than the last one plus one, lower ones
//! included, as missed pushes and refetch the board.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::broadcast;

use super::model::ItemCard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    ItemUpserted,
    ItemMoved,
    ItemRemoved,
    ColumnsChanged,
    SettingsChanged,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoardChange {
    pub project_id: String,
    pub seq: u64,
    pub kind: ChangeKind,
    pub item_id: String,
    pub card: Option<ItemCard>,
    pub from_column: Option<String>,
}

/// How many changes a slow connection may fall behind before it is told to
/// resync instead.
const BACKLOG: usize = 1024;

/// The card to push for a change that has committed. A failed read must not
/// lose the push (ARCH-R8 F2): the change is published without its card, and
/// clients refetch that item.
pub fn card_after_commit(db: &crate::db::Db, item_id: &str) -> Option<ItemCard> {
    db.board_read(|t| t.card(item_id)).unwrap_or_else(|e| {
        tracing::warn!(item_id, error = %e, "card read after commit failed; pushing without it");
        None
    })
}

#[derive(Clone)]
pub struct BoardFeed {
    seqs: Arc<Mutex<HashMap<String, u64>>>,
    tx: broadcast::Sender<Arc<BoardChange>>,
}

impl Default for BoardFeed {
    fn default() -> Self {
        Self::with_backlog(BACKLOG)
    }
}

impl BoardFeed {
    /// A feed that lets a connection fall `backlog` changes behind; tests
    /// use a small one to reach the resync path.
    pub fn with_backlog(backlog: usize) -> Self {
        Self {
            seqs: Arc::default(),
            tx: broadcast::channel(backlog).0,
        }
    }
}

impl BoardFeed {
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<BoardChange>> {
        self.tx.subscribe()
    }

    /// The project's last published sequence number, 0 before any change.
    pub fn seq(&self, project_id: &str) -> u64 {
        self.lock().get(project_id).copied().unwrap_or(0)
    }

    /// Hold the feed for one board write and its publish. Every board
    /// writer takes this before its transaction, the MCP tools included.
    pub fn writer(&self) -> FeedWriter<'_> {
        FeedWriter {
            seqs: self.lock(),
            tx: &self.tx,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, u64>> {
        self.seqs.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct FeedWriter<'a> {
    seqs: MutexGuard<'a, HashMap<String, u64>>,
    tx: &'a broadcast::Sender<Arc<BoardChange>>,
}

/// One change to publish; `seq` is assigned on publishing.
pub struct Change<'a> {
    pub project_id: &'a str,
    pub kind: ChangeKind,
    pub item_id: &'a str,
    pub card: Option<ItemCard>,
    pub from_column: Option<String>,
}

impl FeedWriter<'_> {
    /// Number the change and send it to every watcher; returns its number.
    pub fn publish(&mut self, change: Change<'_>) -> u64 {
        let seq = self.seqs.entry(change.project_id.to_string()).or_insert(0);
        *seq += 1;
        // No receiver is no watcher, which is fine.
        let _ = self.tx.send(Arc::new(BoardChange {
            project_id: change.project_id.to_string(),
            seq: *seq,
            kind: change.kind,
            item_id: change.item_id.to_string(),
            card: change.card,
            from_column: change.from_column,
        }));
        *seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change<'a>(project_id: &'a str, item_id: &'a str) -> Change<'a> {
        Change {
            project_id,
            kind: ChangeKind::ItemUpserted,
            item_id,
            card: None,
            from_column: None,
        }
    }

    #[test]
    fn each_project_counts_its_own_changes_from_one() {
        let feed = BoardFeed::default();
        let mut rx = feed.subscribe();
        assert_eq!(feed.seq("p1"), 0);
        let mut w = feed.writer();
        assert_eq!(w.publish(change("p1", "H-001")), 1);
        assert_eq!(w.publish(change("p2", "X-001")), 1);
        assert_eq!(w.publish(change("p1", "H-002")), 2);
        drop(w);
        assert_eq!((feed.seq("p1"), feed.seq("p2")), (2, 1));
        let got: Vec<(String, u64)> = (0..3)
            .map(|_| {
                let c = rx.try_recv().expect("published");
                (c.item_id.clone(), c.seq)
            })
            .collect();
        assert_eq!(
            got,
            [
                ("H-001".into(), 1),
                ("X-001".into(), 1),
                ("H-002".into(), 2)
            ]
        );
    }

    #[test]
    fn a_reader_waits_for_a_writer_to_publish() {
        let feed = BoardFeed::default();
        let mut w = feed.writer();
        let reader = {
            let feed = feed.clone();
            std::thread::spawn(move || feed.seq("p"))
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        w.publish(change("p", "H-001"));
        drop(w);
        assert_eq!(reader.join().expect("reader"), 1);
    }
}
