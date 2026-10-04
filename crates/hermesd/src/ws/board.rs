//! The board's WS surface (H-020 §1.5): typed `BoardRequest`s in binary
//! frames, served by the board service with the connection's grants, and
//! `board_event` pushes to the connections watching a project. The adapter
//! holds no rule: moves go through `board::moves`, reads through one
//! `board_read` snapshot each.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use bus::contract::board::{self as c, board_request::Request, board_response::Response};
use bus::Capability;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::binary::{self, Frame};
use super::Conn;
use crate::actor::Actor;
use crate::board::feed::{card_after_commit, BoardChange, BoardFeed, Change, ChangeKind};
use crate::board::moves::{self, MoveRequest, Moved};

mod reads;

/// A request refused before the board service answered it: no grant, an
/// unknown item, bad arguments. Sent as the envelope's `Error`.
pub(super) struct Refusal {
    code: &'static str,
    message: String,
}

fn refuse(code: &'static str, message: impl Into<String>) -> Refusal {
    Refusal {
        code,
        message: message.into(),
    }
}

impl From<anyhow::Error> for Refusal {
    fn from(e: anyhow::Error) -> Self {
        tracing::warn!(error = %e, "board request failed");
        refuse("internal", e.to_string())
    }
}

/// The projects a connection watches, and the task forwarding their pushes.
#[derive(Default)]
pub(super) struct Watch {
    projects: Arc<Mutex<HashSet<String>>>,
    task: Option<JoinHandle<()>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn lock(projects: &Mutex<HashSet<String>>) -> MutexGuard<'_, HashSet<String>> {
    projects.lock().unwrap_or_else(|e| e.into_inner())
}

/// What answering a request needs: the response, or none when the handler
/// already sent it itself (`board_watch` orders it before its pushes).
type Reply = Result<Option<Response>, Refusal>;

impl Conn {
    /// Answer one binary frame.
    pub(super) fn binary_frame(&mut self, bytes: &[u8]) {
        let reply = match binary::decode(bytes) {
            Frame::Refused(error) => error,
            Frame::Board(req_id, request) => match self.board(req_id, request) {
                Ok(Some(response)) => binary::response(
                    req_id,
                    c::BoardResponse {
                        response: Some(response),
                    },
                ),
                Ok(None) => return,
                Err(r) => binary::error(req_id, r.code, r.message),
            },
        };
        let _ = self.bin.send(reply);
    }

    fn board(&mut self, req_id: u64, request: c::BoardRequest) -> Reply {
        let Some(request) = request.request else {
            return Err(refuse("invalid_request", "empty board request"));
        };
        let cap = match request {
            Request::BoardGet(_)
            | Request::ItemGet(_)
            | Request::ItemHistory(_)
            | Request::ItemQuery(_)
            | Request::ItemMoveCheck(_)
            | Request::BoardWatch(_)
            | Request::BoardUnwatch(_) => Capability::Read,
            _ => Capability::Control,
        };
        if !self.caps.contains(&cap) {
            return Err(refuse(
                "forbidden",
                format!(
                    "this board request requires the {} capability",
                    cap.as_str()
                ),
            ));
        }
        Ok(Some(match request {
            Request::BoardGet(r) => Response::Board(self.snapshot(&r.project_id)?),
            Request::BoardWatch(r) => {
                self.board_watch(req_id, &r.project_id)?;
                return Ok(None);
            }
            Request::BoardUnwatch(r) => {
                lock(&self.watch.projects).remove(&r.project_id);
                Response::Unwatched(c::BoardUnwatched {
                    project_id: r.project_id,
                })
            }
            Request::ItemGet(r) => Response::Item(self.item_get(&r.id)?),
            Request::ItemHistory(r) => Response::History(self.item_history(&r)?),
            Request::ItemQuery(r) => Response::Items(self.item_query(&r)?),
            Request::ItemMoveCheck(r) => Response::MoveCheck(self.item_move_check(&r.id)?),
            Request::ItemMove(r) => Response::Moved(self.item_move(&r)?),
            // Bots make these edits over MCP (B5); the owner's item drawer
            // (U4) serves them here next.
            Request::ItemCreate(_)
            | Request::ItemUpdate(_)
            | Request::ItemComment(_)
            | Request::ItemLink(_)
            | Request::ItemUnlink(_)
            | Request::ItemBlock(_)
            | Request::ItemUnblock(_)
            | Request::ItemAssign(_)
            | Request::ItemRank(_)
            | Request::ItemCheckAc(_) => {
                return Err(refuse(
                    "unsupported",
                    "item edits aren't served over WebSocket yet",
                ))
            }
        }))
    }

    /// The owner, on the owner token or through a device.
    fn actor(&self) -> Actor<'_> {
        self.device_id.as_deref().map_or(Actor::User, Actor::Device)
    }

    /// Run the move through the guards in one transaction, holding the feed
    /// so its push is numbered in commit order.
    fn item_move(&self, r: &c::ItemMove) -> Result<c::MoveResult, Refusal> {
        use c::move_result::Outcome;
        let db = &self.app.db;
        let mut feed = self.app.board.writer();
        let (project_id, from) = db
            .board_read(|t| Ok(t.item_project(&r.id)?.zip(t.item(&r.id)?)))?
            .map(|(project, item)| (project, item.column_key))
            .ok_or_else(|| not_found(&r.id))?;
        let request = MoveRequest {
            id: &r.id,
            to: &r.to,
            expected_version: r.expected_version,
            reason: r.reason.as_deref(),
            override_reason: r.override_reason.as_deref(),
        };
        let outcome = match moves::item_move(db, &request, &self.actor())? {
            Moved::Done(item) => {
                let card = card_after_commit(db, &item.id);
                feed.publish(Change {
                    project_id: &project_id,
                    kind: ChangeKind::ItemMoved,
                    item_id: &item.id,
                    card,
                    from_column: Some(from),
                });
                Outcome::Done((*item).into())
            }
            Moved::Refused(unmet) => Outcome::Refused(c::MoveRefused {
                unmet: unmet.into_iter().map(Into::into).collect(),
            }),
            Moved::Conflict(item) => Outcome::Conflict((*item).into()),
        };
        Ok(c::MoveResult {
            outcome: Some(outcome),
        })
    }

    /// Start forwarding the project's pushes and answer with the snapshot
    /// they continue from. The answer is queued while the watch list is
    /// held, so no push for the project can overtake it.
    fn board_watch(&mut self, req_id: u64, project_id: &str) -> Result<(), Refusal> {
        self.forward_pushes();
        let projects = self.watch.projects.clone();
        let mut watched = lock(&projects);
        let snapshot = self.snapshot(project_id)?;
        watched.insert(project_id.to_string());
        let _ = self.bin.send(binary::response(
            req_id,
            c::BoardResponse {
                response: Some(Response::Board(snapshot)),
            },
        ));
        Ok(())
    }

    /// Subscribe once per connection, before the first snapshot's `seq` is
    /// read, so no change after it can be missed.
    fn forward_pushes(&mut self) {
        if self.watch.task.is_some() {
            return;
        }
        let rx = self.app.board.subscribe();
        let task = forward(
            rx,
            self.app.board.clone(),
            self.watch.projects.clone(),
            self.bin.clone(),
        );
        self.watch.task = Some(tokio::spawn(task));
    }
}

async fn forward(
    mut rx: tokio::sync::broadcast::Receiver<Arc<BoardChange>>,
    feed: BoardFeed,
    projects: Arc<Mutex<HashSet<String>>>,
    out: mpsc::UnboundedSender<Vec<u8>>,
) {
    loop {
        let frames: Vec<Vec<u8>> = match rx.recv().await {
            Ok(change) => {
                let watched = lock(&projects);
                if !watched.contains(&change.project_id) {
                    continue;
                }
                vec![binary::push(push(&change))]
            }
            // Too far behind: say so per watched project rather than send a
            // feed with holes in it.
            Err(RecvError::Lagged(skipped)) => {
                tracing::warn!(
                    skipped,
                    "board push feed lagged; asking the client to resync"
                );
                lock(&projects)
                    .iter()
                    .map(|project_id| binary::push(resync(project_id, feed.seq(project_id))))
                    .collect()
            }
            Err(RecvError::Closed) => break,
        };
        for frame in frames {
            if out.send(frame).is_err() {
                return;
            }
        }
    }
}

fn push(change: &BoardChange) -> c::BoardPush {
    let kind = match change.kind {
        ChangeKind::ItemUpserted => c::BoardEventKind::ItemUpserted,
        ChangeKind::ItemMoved => c::BoardEventKind::ItemMoved,
        ChangeKind::ItemRemoved => c::BoardEventKind::ItemRemoved,
        ChangeKind::ColumnsChanged => c::BoardEventKind::ColumnsChanged,
        ChangeKind::SettingsChanged => c::BoardEventKind::SettingsChanged,
    };
    event(c::BoardEvent {
        project_id: change.project_id.clone(),
        seq: change.seq,
        kind: kind as i32,
        item_id: change.item_id.clone(),
        card: change.card.clone().map(Into::into),
        from_column: change.from_column.clone(),
    })
}

fn resync(project_id: &str, seq: u64) -> c::BoardPush {
    event(c::BoardEvent {
        project_id: project_id.to_string(),
        seq,
        kind: c::BoardEventKind::Resync as i32,
        ..Default::default()
    })
}

fn event(event: c::BoardEvent) -> c::BoardPush {
    c::BoardPush {
        push: Some(c::board_push::Push::BoardEvent(event)),
    }
}

fn not_found(id: &str) -> Refusal {
    refuse("not_found", format!("no item {id}"))
}

#[cfg(test)]
mod tests {
    use prost::Message;

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

    fn board_event(frame: &[u8]) -> c::BoardEvent {
        use bus::contract::wire::{envelope::Body, Envelope};
        match Envelope::decode(frame).expect("an envelope").body {
            Some(Body::BoardPush(c::BoardPush {
                push: Some(c::board_push::Push::BoardEvent(event)),
            })) => event,
            other => panic!("not a board event: {other:?}"),
        }
    }

    /// A connection that falls further behind than the feed keeps is told to
    /// resync each project it watches, at the project's current seq, and then
    /// gets the changes the feed still holds (ARCH-R8 F4).
    #[tokio::test]
    async fn a_lagging_connection_is_told_to_resync() {
        let feed = BoardFeed::with_backlog(2);
        let rx = feed.subscribe();
        let mut w = feed.writer();
        for item in ["H-1", "H-2", "H-3", "H-4", "H-5"] {
            w.publish(change("p", item));
        }
        drop(w);
        let projects = Arc::new(Mutex::new(HashSet::from(["p".to_string()])));
        let (out, mut frames) = mpsc::unbounded_channel();
        let task = tokio::spawn(forward(rx, feed.clone(), projects, out));

        let first = board_event(&frames.recv().await.expect("a frame"));
        assert_eq!(first.kind, c::BoardEventKind::Resync as i32);
        assert_eq!((first.project_id.as_str(), first.seq), ("p", 5));
        let kept: Vec<(String, u64)> = [frames.recv().await, frames.recv().await]
            .into_iter()
            .map(|f| board_event(&f.expect("a frame")))
            .map(|e| (e.item_id, e.seq))
            .collect();
        assert_eq!(kept, [("H-4".to_string(), 4), ("H-5".to_string(), 5)]);
        task.abort();
    }
}
