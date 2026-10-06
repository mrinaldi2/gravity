//! `projects_overview` (H-128 §2, rev 2): one row per project, sorted by how
//! much it needs the owner, across linked computers.
//!
//! The read never waits on a peer. It answers from local data and the last
//! good answer of each peer; peers are asked in the background, one batched
//! `project_attention` per peer, on its `project_attention_changed`, on
//! link-up, and every minute while a client has looked in the last five.
//! When an answer changes a row, clients get `projects_overview_changed`.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bus::contract::home::{project_attention::Part, source::State};
use chrono::{DateTime, Utc};

mod build;
mod detail;
mod liveness;
mod part;
mod peers;
mod pin;
#[cfg(test)]
mod tests;

pub use build::overview;
pub use detail::{attention_rows, dismiss};
pub use liveness::spawn;
pub use part::part;
pub use peers::{link_down, link_up, receive_changed, refresh, serve};
pub use pin::{pin, serve_pin};

/// How long a peer may take to answer `project_attention`.
const PEER_TIMEOUT: Duration = Duration::from_secs(3);
/// How often peers are asked again while the overview is watched.
const REFRESH_EVERY: Duration = Duration::from_secs(60);
/// A client's fetch keeps the overview watched this long.
const WATCHED_FOR: Duration = Duration::from_secs(5 * 60);
/// Changes are pushed at most this often.
const DEBOUNCE: Duration = Duration::from_secs(2);

/// What a peer last answered, and how its last request went.
#[derive(Debug, Clone)]
struct PeerEntry {
    /// Of the last request; `Unspecified` until one was made.
    state: State,
    /// When the parts in use were answered; none before the first answer.
    as_of: Option<DateTime<Utc>>,
    /// The last good answer, in this computer's project ids. Kept until
    /// replaced: `as_of` tells its age.
    parts: Vec<Part>,
}

impl Default for PeerEntry {
    fn default() -> Self {
        Self {
            state: State::Unspecified,
            as_of: None,
            parts: Vec::new(),
        }
    }
}

/// A peer's refresh: running, and asked again while it ran.
#[derive(Debug, Clone, Copy, Default)]
struct InFlight {
    again: bool,
}

/// The overview's state: peers' last answers and what keeps them fresh.
pub struct Overview {
    peers: Mutex<HashMap<String, PeerEntry>>,
    in_flight: Mutex<HashMap<String, InFlight>>,
    watched_at: Mutex<Option<Instant>>,
    /// Since when each bot has been waiting for the owner.
    waiting: Mutex<HashMap<String, DateTime<Utc>>>,
    started_at: DateTime<Utc>,
}

impl Default for Overview {
    fn default() -> Self {
        Self {
            peers: Mutex::default(),
            in_flight: Mutex::default(),
            watched_at: Mutex::default(),
            waiting: Mutex::default(),
            started_at: Utc::now(),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Overview {
    /// When this daemon started: the age of what has been true since.
    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    /// Since when a bot has been waiting for the owner, as far as this
    /// daemon has seen.
    pub fn waiting_since(&self, bot_id: &str) -> DateTime<Utc> {
        lock(&self.waiting)
            .get(bot_id)
            .copied()
            .unwrap_or(self.started_at)
    }

    fn set_waiting(&self, bot_id: &str, since: Option<DateTime<Utc>>) {
        let mut waiting = lock(&self.waiting);
        match since {
            Some(at) => {
                waiting.entry(bot_id.to_string()).or_insert(at);
            }
            None => {
                waiting.remove(bot_id);
            }
        }
    }

    /// A client read the overview: peers stay refreshed for a while.
    fn watched_now(&self) {
        *lock(&self.watched_at) = Some(Instant::now());
    }

    fn is_watched(&self) -> bool {
        lock(&self.watched_at).is_some_and(|at| at.elapsed() < WATCHED_FOR)
    }

    fn entry(&self, peer_id: &str) -> PeerEntry {
        lock(&self.peers).get(peer_id).cloned().unwrap_or_default()
    }

    /// Stores how a request went; `true` when the state or the parts
    /// changed. A fresher `as_of` alone changes no row.
    fn store(&self, peer_id: &str, entry: PeerEntry) -> bool {
        let mut peers = lock(&self.peers);
        let old = peers.insert(peer_id.to_string(), entry.clone());
        old.is_none_or(|old| old.state != entry.state || old.parts != entry.parts)
    }

    fn forget(&self, peer_id: &str) {
        lock(&self.peers).remove(peer_id);
    }

    /// Claims the peer's refresh; `false` when one is running, which will
    /// then run once more.
    fn begin(&self, peer_id: &str) -> bool {
        let mut running = lock(&self.in_flight);
        match running.get_mut(peer_id) {
            Some(flight) => {
                flight.again = true;
                false
            }
            None => {
                running.insert(peer_id.to_string(), InFlight::default());
                true
            }
        }
    }

    /// Ends one pass of the refresh; `true` when it must run again.
    fn finish(&self, peer_id: &str) -> bool {
        let mut running = lock(&self.in_flight);
        match running.get_mut(peer_id) {
            Some(flight) if flight.again => {
                flight.again = false;
                true
            }
            _ => {
                running.remove(peer_id);
                false
            }
        }
    }
}
