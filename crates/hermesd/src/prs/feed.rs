//! What changed about a project's PRs (H-273; H-261 §9 pushes). The board
//! transaction that writes a PR, a review, a comment, a check or the merge
//! queue notes it, and the notes are published here once it commits, so no
//! push names a change that rolled back and none is missed by a new caller.
//! The WS sends them as `PrPush`es; the board's home relays them to linked
//! computers.

use std::sync::Arc;

use bus::contract::pr::{self as p, pr_push::Push};
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrChange {
    /// `pr_updated`: anything `pr_get` shows for the PR.
    Pr { project_id: String, number: u32 },
    /// `check_updated`: a check on a commit.
    Check {
        project_id: String,
        sha: String,
        name: String,
    },
    /// `merge_queue_changed`.
    Queue { project_id: String },
}

impl PrChange {
    pub fn project_id(&self) -> &str {
        match self {
            Self::Pr { project_id, .. }
            | Self::Check { project_id, .. }
            | Self::Queue { project_id } => project_id,
        }
    }

    /// The same change in another project: the peer's own, after a relay.
    pub fn in_project(&self, project_id: &str) -> Self {
        let mut out = self.clone();
        match &mut out {
            Self::Pr { project_id: p, .. }
            | Self::Check { project_id: p, .. }
            | Self::Queue { project_id: p } => *p = project_id.to_string(),
        }
        out
    }

    pub fn to_push(&self) -> p::PrPush {
        let push = match self.clone() {
            Self::Pr { project_id, number } => Push::PrUpdated(p::PrUpdated { project_id, number }),
            Self::Check {
                project_id,
                sha,
                name,
            } => Push::CheckUpdated(p::CheckUpdated {
                project_id,
                sha,
                name,
            }),
            Self::Queue { project_id } => {
                Push::MergeQueueChanged(p::MergeQueueChanged { project_id })
            }
        };
        p::PrPush { push: Some(push) }
    }

    pub fn from_push(push: &p::PrPush) -> Option<Self> {
        Some(match push.push.clone()? {
            Push::PrUpdated(u) => Self::Pr {
                project_id: u.project_id,
                number: u.number,
            },
            Push::CheckUpdated(u) => Self::Check {
                project_id: u.project_id,
                sha: u.sha,
                name: u.name,
            },
            Push::MergeQueueChanged(u) => Self::Queue {
                project_id: u.project_id,
            },
            Push::CleanupUpdated(_) => return None,
        })
    }
}

/// Committed PR changes, to every subscriber.
#[derive(Clone)]
pub struct PrFeed {
    tx: broadcast::Sender<Arc<PrChange>>,
}

impl Default for PrFeed {
    fn default() -> Self {
        Self {
            tx: broadcast::channel(1024).0,
        }
    }
}

impl PrFeed {
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<PrChange>> {
        self.tx.subscribe()
    }

    /// Each distinct change once, in the order first noted.
    pub fn publish(&self, changes: Vec<PrChange>) {
        let mut sent: Vec<&PrChange> = Vec::new();
        for change in &changes {
            if sent.contains(&change) {
                continue;
            }
            sent.push(change);
            let _ = self.tx.send(Arc::new(change.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_published_once_and_round_trips_as_a_push() {
        let feed = PrFeed::default();
        let mut rx = feed.subscribe();
        let pr = PrChange::Pr {
            project_id: "p".into(),
            number: 1,
        };
        let queue = PrChange::Queue {
            project_id: "p".into(),
        };
        feed.publish(vec![pr.clone(), queue.clone(), pr.clone()]);
        assert_eq!(*rx.try_recv().unwrap(), pr);
        assert_eq!(*rx.try_recv().unwrap(), queue);
        assert!(rx.try_recv().is_err());
        let check = PrChange::Check {
            project_id: "p".into(),
            sha: "abc".into(),
            name: "rust".into(),
        };
        assert_eq!(PrChange::from_push(&check.to_push()), Some(check.clone()));
        assert_eq!(check.in_project("q").project_id(), "q");
    }
}
