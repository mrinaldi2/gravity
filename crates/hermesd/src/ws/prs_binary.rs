//! The PR surface over binary frames (H-273; H-261 §9): `pr_list`, `pr_get`,
//! `pr_diff` and `pr_comments` (read), and `check_rerun` (control, the
//! owner's from a device or ticket). The owner's reviews, comments, flag and
//! settings stay JSON requests (`ws::prs`, H-269).
//!
//! A connection's first PR request for a project starts its pushes for that
//! project: `pr_updated`, `check_updated` and `merge_queue_changed`, as the
//! board transactions behind them commit (`prs::feed`).
//!
//! Off the board's home (B9) the reads go to the home as `pr_read` and come
//! back in this computer's project; its pushes arrive through the home's
//! relay. Writes stay on the home: a bot's go through `board_call` as for
//! every board tool, and the owner's re-run is forwarded there as
//! `pr_owner` with how the owner proved it here (H-285).

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use bus::contract::pr::{self as p, pr_request::Request};
use bus::Capability;
use serde_json::json;
use tokio::sync::broadcast::{error::RecvError, Receiver};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::binary;
use super::board::{refuse, Refusal};
use super::Conn;
use crate::peer::board::home_name;
use crate::peer::PeerError;
use crate::prs::check_rerun::{self, Asker};
use crate::prs::feed::PrChange;
use crate::prs::{read, wire};

/// The projects whose PR pushes a connection gets, and the task sending them.
#[derive(Default)]
pub(super) struct PrWatch {
    projects: Arc<Mutex<HashSet<String>>>,
    task: Option<JoinHandle<()>>,
}

impl Drop for PrWatch {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn lock(projects: &Mutex<HashSet<String>>) -> MutexGuard<'_, HashSet<String>> {
    projects.lock().unwrap_or_else(|e| e.into_inner())
}

/// An error as the envelope's `Error`, with the code its kind names.
fn refusal(e: &anyhow::Error) -> Refusal {
    let code = crate::decisions::error_code(e).unwrap_or_else(|| {
        let peer = crate::peer::error_code(e, "internal");
        [
            "forbidden",
            "not_found",
            "not_linked",
            "no_board",
            "unavailable",
        ]
        .into_iter()
        .find(|c| *c == peer)
        .unwrap_or("internal")
    });
    refuse(code, e.to_string())
}

impl Conn {
    pub(super) fn pr_frame(&mut self, req_id: u64, request: p::PrRequest) -> Result<(), Refusal> {
        let Some(request) = request.request else {
            return Err(refuse("invalid_request", "empty PR request"));
        };
        let cap = match request {
            ref r if read::is_read(r) => Capability::Read,
            Request::CheckRerun(_) => Capability::Control,
            _ => {
                return Err(refuse(
                    "unsupported",
                    "this PR request is a JSON request on this daemon: {type, req_id, …}",
                ))
            }
        };
        if !self.caps.contains(&cap) {
            return Err(refuse(
                "forbidden",
                format!("this PR request requires the {} capability", cap.as_str()),
            ));
        }
        let project = read::project_of(&request).unwrap_or_default().to_string();
        if project.is_empty() {
            return Err(refuse("invalid_request", "'project_id' is required"));
        }
        self.watch_prs(&project);
        if let Some(home) = self.app.board_mirror.home_peer(&project) {
            if let Request::CheckRerun(r) = request {
                return self.forward_rerun(req_id, project, home, r);
            }
            self.forward_pr_read(req_id, project, home, request);
            return Ok(());
        }
        if let Request::CheckRerun(r) = request {
            let check = self.check_rerun(&r).map_err(|e| refusal(&e))?;
            let response = p::PrResponse {
                response: Some(p::pr_response::Response::Check(check)),
            };
            let _ = self.bin.send(binary::pr_response(req_id, response));
            return Ok(());
        }
        let app = self.app.clone();
        self.spawn_frame(req_id, "pr_read", async move {
            match tokio::task::spawn_blocking(move || read::serve(&app, request)).await {
                Ok(Ok(response)) => binary::pr_response(
                    req_id,
                    p::PrResponse {
                        response: Some(response),
                    },
                ),
                Ok(Err(e)) => {
                    let r = refusal(&e);
                    binary::error(req_id, r.code, r.message)
                }
                Err(e) => binary::error(req_id, "internal", e.to_string()),
            }
        });
        Ok(())
    }

    /// The owner's re-run: from a paired device or the app's ticket only.
    fn check_rerun(&self, r: &p::CheckRerunRequest) -> anyhow::Result<p::CheckRun> {
        if self.owner_proof().is_none() {
            return Err(crate::decisions::forbidden(
                "only the owner's app or a paired device re-runs a check",
            ));
        }
        let run = check_rerun::rerun(
            &self.app,
            &r.project_id,
            r.sha.trim(),
            r.name.trim(),
            &Asker::Owner,
        )?;
        Ok(wire::check(self.app.as_ref(), &run.to_json()))
    }

    /// The owner's re-run for a board kept on `home` (H-285): forwarded as
    /// `pr_owner` with how the owner proved it here.
    fn forward_rerun(
        &self,
        req_id: u64,
        project: String,
        home: String,
        r: p::CheckRerunRequest,
    ) -> Result<(), Refusal> {
        // Not taken from a linked computer yet (H-285 must-fix).
        if !crate::peer::owner_trust::TRUST_FORWARDED_OWNER_ACTS {
            let name = home_name(&self.app, &home);
            return Err(refuse(
                "forbidden",
                crate::peer::owner_trust::approve_elsewhere(&name),
            ));
        }
        let via = match self.owner_proof() {
            Some(crate::db::OwnerProof::Device { .. }) => "device",
            Some(crate::db::OwnerProof::Ticket) => "ticket",
            _ => {
                return Err(refuse(
                    "forbidden",
                    "only the owner's app or a paired device re-runs a check",
                ))
            }
        };
        let app = self.app.clone();
        self.spawn_frame(req_id, "check_rerun", async move {
            let frame = json!({
                "type": "pr_owner", "project_id": project, "kind": "check_rerun",
                "request": { "sha": r.sha, "name": r.name }, "via": via,
            });
            match app.peers.request(&home, frame).await {
                Ok(value) => {
                    let mut check = value["check"].clone();
                    check["project_id"] = json!(project);
                    binary::pr_response(
                        req_id,
                        p::PrResponse {
                            response: Some(p::pr_response::Response::Check(wire::check(
                                app.as_ref(),
                                &check,
                            ))),
                        },
                    )
                }
                Err(PeerError::Offline) => binary::error(
                    req_id,
                    "unavailable",
                    format!("{} can't be reached right now", home_name(&app, &home)),
                ),
                Err(e) => {
                    let r = refusal(&anyhow::Error::from(e));
                    binary::error(req_id, r.code, r.message)
                }
            }
        });
        Ok(())
    }

    /// Sends a read to the board's home, answering `req_id` in this
    /// computer's project once the home has.
    fn forward_pr_read(&self, req_id: u64, project: String, home: String, request: Request) {
        let app = self.app.clone();
        self.spawn_frame(req_id, "pr_read", async move {
            let frame = json!({
                "type": "pr_read", "project_id": project,
                "request": p::PrRequest { request: Some(request) },
            });
            match app.peers.request(&home, frame).await {
                Ok(value) => {
                    match serde_json::from_value::<p::PrResponse>(value["response"].clone()) {
                        Ok(mut response) => {
                            read::into_project(&mut response, &project);
                            binary::pr_response(req_id, response)
                        }
                        Err(_) => binary::error(
                            req_id,
                            "internal",
                            "unreadable answer from the board's home".into(),
                        ),
                    }
                }
                Err(PeerError::Offline) => binary::error(
                    req_id,
                    "unavailable",
                    format!(
                        "This project's PRs are on the board {} holds, which can't be reached \
                         right now.",
                        home_name(&app, &home)
                    ),
                ),
                Err(e) => {
                    let r = refusal(&anyhow::Error::from(e));
                    binary::error(req_id, r.code, r.message)
                }
            }
        });
    }

    /// Starts the project's pushes, subscribing before the first answer so
    /// no change after it is missed.
    fn watch_prs(&mut self, project: &str) {
        lock(&self.pr_watch.projects).insert(project.to_string());
        if self.pr_watch.task.is_none() {
            let rx = self.app.db.pr_feed.subscribe();
            let task = forward(rx, self.pr_watch.projects.clone(), self.bin.clone());
            self.pr_watch.task = Some(tokio::spawn(task));
        }
    }
}

async fn forward(
    mut rx: Receiver<Arc<PrChange>>,
    projects: Arc<Mutex<HashSet<String>>>,
    out: mpsc::UnboundedSender<Vec<u8>>,
) {
    loop {
        match rx.recv().await {
            Ok(change) => {
                if !lock(&projects).contains(change.project_id()) {
                    continue;
                }
                if out.send(binary::pr_push(change.to_push())).is_err() {
                    return;
                }
            }
            Err(RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "PR push feed lagged; some pushes were dropped");
            }
            Err(RecvError::Closed) => return,
        }
    }
}
