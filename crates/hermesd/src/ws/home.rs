//! The projects home's WS surface (H-128 rev 2): `projects_overview`,
//! `attention_rows`, `attention_dismiss`, `project_pin` and the owner
//! threads, typed `HomeRequest`s in binary frames and the same messages as
//! proto3 JSON (proto field names) on the JSON protocol. The overview
//! answers at once; it never waits on a peer. A stand-in's thread is read
//! from its own computer, so the thread requests answer from a task.

use std::future::Future;

use bus::contract::home::{
    home_request::Request, home_response::Response, AttentionDismissRequest, AttentionRowsRequest,
    HomeRequest, HomeResponse, OwnerThreadGetRequest, OwnerThreadReadRequest, ProjectPinRequest,
    ProjectsOverviewRequest,
};
use bus::Capability;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use super::board::{refuse, Refusal};
use super::{binary, Conn};
use crate::owner_threads;

/// A JSON request's fields as the typed message, `type` and `req_id` aside.
fn typed<T: DeserializeOwned>(req: &Value) -> anyhow::Result<T> {
    let mut fields = req.clone();
    if let Some(map) = fields.as_object_mut() {
        map.remove("type");
        map.remove("req_id");
    }
    serde_json::from_value(fields).map_err(|e| crate::decisions::invalid(e.to_string()))
}

/// A failed thread request: its own code, else the bot's computer is out
/// of reach.
fn unavailable(e: &anyhow::Error) -> Refusal {
    match crate::decisions::error_code(e) {
        Some(code) => refuse(code, e.to_string()),
        None => refuse("unavailable", format!("{e:#}")),
    }
}

impl Conn {
    pub(super) fn projects_overview(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let r: ProjectsOverviewRequest = typed(req)?;
        let overview = crate::overview::overview(&self.app, &r.project_ids)?;
        self.send(json!({ "type": "projects_overview", "req_id": req_id, "overview": overview }));
        Ok(())
    }

    pub(super) fn attention_rows(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let r: AttentionRowsRequest = typed(req)?;
        let rows = crate::overview::attention_rows(&self.app, &r.project_id)?;
        self.send(json!({ "type": "attention_rows", "req_id": req_id, "attention_rows": rows }));
        Ok(())
    }

    pub(super) fn attention_dismiss(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let r: AttentionDismissRequest = typed(req)?;
        let done = crate::overview::dismiss(&self.app, &r.id)?;
        self.send(json!({ "type": "attention_dismissed", "req_id": req_id, "id": done.id }));
        Ok(())
    }

    /// `project_pin` (control, D7): `{type: "project_pinned", project_id, pinned}`.
    pub(super) fn project_pin(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let r: ProjectPinRequest = typed(req)?;
        let pinned = crate::overview::pin(&self.app, &r.project_id, r.pinned)?;
        self.send(json!({
            "type": "project_pinned", "req_id": req_id,
            "project_id": pinned.project_id, "pinned": pinned.pinned,
        }));
        Ok(())
    }

    /// `owner_threads` (read, D6): `{type, owner_threads: OwnerThreads}`.
    pub(super) fn owner_threads(&self, req_id: &Value) -> anyhow::Result<()> {
        let app = self.app.clone();
        self.thread_answer(req_id, "owner_threads", owner_threads::threads(app));
        Ok(())
    }

    /// `owner_thread_get` (read): `{type: "owner_thread", owner_thread: OwnerThreadPage}`.
    pub(super) fn owner_thread_get(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let r: OwnerThreadGetRequest = typed(req)?;
        let app = self.app.clone();
        self.thread_answer(req_id, "owner_thread", async move {
            owner_threads::get(app, &r.bot_id, r.before_num, r.limit).await
        });
        Ok(())
    }

    /// `owner_thread_read` (read): `{type: "owner_thread_marked", owner_thread_marked}`.
    pub(super) fn owner_thread_read(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let r: OwnerThreadReadRequest = typed(req)?;
        let app = self.app.clone();
        self.thread_answer(req_id, "owner_thread_marked", async move {
            owner_threads::read(app, &r.bot_id, r.up_to_num).await
        });
        Ok(())
    }

    /// Answers `{type: kind, <kind>: answer}` once `work` is done.
    fn thread_answer<T: Serialize>(
        &self,
        req_id: &Value,
        kind: &'static str,
        work: impl Future<Output = anyhow::Result<T>> + Send + 'static,
    ) {
        let (out, req_id) = (self.out.clone(), req_id.clone());
        tokio::spawn(async move {
            let reply = match work.await {
                Ok(answer) => json!({ "type": kind, "req_id": req_id, kind: answer }),
                Err(e) => {
                    let r = unavailable(&e);
                    json!({ "type": "error", "req_id": req_id, "code": r.code, "message": r.message })
                }
            };
            let _ = out.send(reply);
        });
    }

    /// One binary `HomeRequest`, under the grants the JSON requests need.
    /// `None` when a task sends the answer.
    pub(super) fn home(
        &self,
        req_id: u64,
        request: HomeRequest,
    ) -> Result<Option<HomeResponse>, Refusal> {
        let Some(request) = request.request else {
            return Err(refuse("invalid_request", "empty home request"));
        };
        let cap = match request {
            Request::ProjectsOverview(_)
            | Request::AttentionRows(_)
            | Request::OwnerThreads(_)
            | Request::OwnerThreadGet(_)
            | Request::OwnerThreadRead(_) => Capability::Read,
            Request::AttentionDismiss(_) => Capability::Approve,
            Request::ProjectPin(_) => Capability::Control,
        };
        if !self.caps.contains(&cap) {
            return Err(refuse(
                "forbidden",
                format!("this request requires the {} capability", cap.as_str()),
            ));
        }
        let app = self.app.clone();
        let response = match request {
            Request::ProjectsOverview(r) => {
                crate::overview::overview(&app, &r.project_ids).map(Response::ProjectsOverview)
            }
            Request::AttentionRows(r) => {
                crate::overview::attention_rows(&app, &r.project_id).map(Response::AttentionRows)
            }
            Request::AttentionDismiss(r) => {
                crate::overview::dismiss(&app, &r.id).map(Response::AttentionDismissed)
            }
            Request::ProjectPin(r) => {
                crate::overview::pin(&app, &r.project_id, r.pinned).map(Response::ProjectPinned)
            }
            Request::OwnerThreads(_) => {
                self.home_later(req_id, async move {
                    owner_threads::threads(app)
                        .await
                        .map(Response::OwnerThreads)
                });
                return Ok(None);
            }
            Request::OwnerThreadGet(r) => {
                self.home_later(req_id, async move {
                    owner_threads::get(app, &r.bot_id, r.before_num, r.limit)
                        .await
                        .map(Response::OwnerThread)
                });
                return Ok(None);
            }
            Request::OwnerThreadRead(r) => {
                self.home_later(req_id, async move {
                    owner_threads::read(app, &r.bot_id, r.up_to_num)
                        .await
                        .map(Response::OwnerThreadMarked)
                });
                return Ok(None);
            }
        };
        response
            .map(|response| {
                Some(HomeResponse {
                    response: Some(response),
                })
            })
            .map_err(|e| match crate::decisions::error_code(&e) {
                Some(code) => refuse(code, e.to_string()),
                None => Refusal::from(e),
            })
    }

    fn home_later(
        &self,
        req_id: u64,
        work: impl Future<Output = anyhow::Result<Response>> + Send + 'static,
    ) {
        let bin = self.bin.clone();
        tokio::spawn(async move {
            let frame = match work.await {
                Ok(response) => binary::home_response(
                    req_id,
                    HomeResponse {
                        response: Some(response),
                    },
                ),
                Err(e) => {
                    let r = unavailable(&e);
                    binary::error(req_id, r.code, r.message)
                }
            };
            let _ = bin.send(frame);
        });
    }
}
