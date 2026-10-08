//! What the daemon logs about each client connection (H-218): who connected,
//! why it went, a device holding two sockets at once, and a request handler
//! that took too long. A line names a paired device by its name and a short
//! id, and the owner's clients by the name their hello gives; never a token,
//! and never a request's or a message's content.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::app::AppState;

/// A request handler that takes longer than this is logged.
pub(super) const SLOW_HANDLER: Duration = Duration::from_secs(2);

/// Longest name a line quotes, so a hello cannot fill the log.
const MAX_NAME: usize = 48;

/// Why a connection ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reason {
    /// The client closed it, or the link dropped.
    Closed,
    /// The client sent nothing for too long, a hello included.
    Timeout,
    WriterStopped,
    PushFeedStopped,
    /// The hello's credential was invalid or revoked.
    AuthRefused,
    /// The first frame was no hello the daemon speaks.
    BadHello,
    /// The connection's task panicked.
    Panicked,
}

impl Reason {
    fn as_str(self) -> &'static str {
        match self {
            Reason::Closed => "closed",
            Reason::Timeout => "timeout",
            Reason::WriterStopped => "writer stopped",
            Reason::PushFeedStopped => "push feed stopped",
            Reason::AuthRefused => "auth refused",
            Reason::BadHello => "bad hello",
            Reason::Panicked => "panicked",
        }
    }
}

/// Who a connection is, as its log lines name it.
#[derive(Clone, Debug)]
pub(super) struct Who(String);

impl fmt::Display for Who {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Who {
    /// An authenticated connection: the paired device, or the owner's
    /// client by the way it signed in.
    pub(super) fn of(
        app: &AppState,
        device_id: Option<&str>,
        via_ticket: bool,
        hello: &str,
    ) -> Self {
        let client = client_name(hello);
        let who = match device_id {
            Some(id) => {
                let name = app
                    .db
                    .get_device(id)
                    .ok()
                    .flatten()
                    .map(|d| d.name)
                    .unwrap_or_default();
                format!(
                    "device \"{}\" ({}) client \"{client}\"",
                    clean(&name),
                    short(id)
                )
            }
            None if via_ticket => format!("owner app \"{client}\""),
            None => format!("owner token \"{client}\""),
        };
        Self(who)
    }

    /// A connection that never authenticated.
    pub(super) fn unknown(hello: Option<&str>) -> Self {
        Self(format!(
            "unidentified client \"{}\"",
            hello.map(client_name).unwrap_or_default()
        ))
    }
}

/// The name a hello gives its client, cleaned and masked.
fn client_name(hello: &str) -> String {
    serde_json::from_str::<Value>(hello)
        .ok()
        .and_then(|h| h["client"].as_str().map(clean))
        .unwrap_or_default()
}

/// `text` on one line, clipped, with anything token-like masked.
fn clean(text: &str) -> String {
    let line: String = text
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    crate::redact::secrets(&line)
}

/// An id cut short: enough to tell devices apart in a log.
fn short(id: &str) -> String {
    let head: String = id.chars().take(8).collect();
    format!("{head}…")
}

/// Numbers each connection, so the lines of one can be told apart.
static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// The open connections of each paired device, by connection number.
static LIVE: LazyLock<Mutex<HashMap<String, Vec<u64>>>> = LazyLock::new(Default::default);

/// An authenticated connection. Logs its connect when made and its
/// disconnect, with `reason`, when dropped: an unwind included, which
/// leaves the reason at `Panicked`.
pub(super) struct Presence {
    who: Who,
    conn: u64,
    device: Option<String>,
    pub(super) reason: Reason,
}

impl Presence {
    pub(super) fn connected(who: Who, device_id: Option<&str>) -> Self {
        let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
        tracing::info!(conn, client = %who, "client connected");
        if let Some(id) = device_id {
            let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            let open = live.entry(id.to_string()).or_default();
            // Logged, not closed: a client may open its new socket before it
            // drops the old one (H-217's probe does).
            for &other in open.iter() {
                tracing::info!(
                    conn,
                    other,
                    client = %who,
                    reason = "concurrent",
                    "a device opened a second socket"
                );
            }
            open.push(conn);
        }
        Self {
            who,
            conn,
            device: device_id.map(str::to_string),
            reason: Reason::Panicked,
        }
    }

    /// Who this connection is, for lines logged on its behalf.
    pub(super) fn who(&self) -> &Who {
        &self.who
    }

    pub(super) fn conn(&self) -> u64 {
        self.conn
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        if let Some(id) = &self.device {
            let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(open) = live.get_mut(id) {
                open.retain(|&c| c != self.conn);
                if open.is_empty() {
                    live.remove(id);
                }
            }
        }
        tracing::info!(
            conn = self.conn,
            client = %self.who,
            reason = self.reason.as_str(),
            "client disconnected"
        );
    }
}

/// A connection that ended before it authenticated.
pub(super) fn refused(who: &Who, reason: Reason) {
    tracing::info!(client = %who, reason = reason.as_str(), "client disconnected");
}

/// Logs a handler that took longer than [`SLOW_HANDLER`]; `kind` is named
/// only then.
pub(super) fn note_handler(elapsed: Duration, kind: impl FnOnce() -> String) {
    if elapsed > SLOW_HANDLER {
        let kind: String = kind().chars().take(MAX_NAME).collect();
        let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        tracing::warn!(kind, elapsed_ms, "a request handler was slow");
    }
}

#[cfg(test)]
#[path = "presence_tests.rs"]
mod tests;
