//! The projects home's WS surface (H-128 rev 2): `projects_overview`,
//! `attention_rows` and `attention_dismiss`, typed `HomeRequest`s in binary
//! frames and the same messages as proto3 JSON (proto field names) on the
//! JSON protocol. The overview answers at once; it never waits on a peer.

use bus::contract::home::{
    home_request::Request, home_response::Response, AttentionDismissRequest, AttentionRowsRequest,
    HomeRequest, HomeResponse, ProjectsOverviewRequest,
};
use bus::Capability;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::board::{refuse, Refusal};
use super::Conn;

/// A JSON request's fields as the typed message, `type` and `req_id` aside.
fn typed<T: DeserializeOwned>(req: &Value) -> anyhow::Result<T> {
    let mut fields = req.clone();
    if let Some(map) = fields.as_object_mut() {
        map.remove("type");
        map.remove("req_id");
    }
    serde_json::from_value(fields).map_err(|e| crate::decisions::invalid(e.to_string()))
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
        let done = crate::overview::dismiss(&r.id)?;
        self.send(json!({ "type": "attention_dismissed", "req_id": req_id, "id": done.id }));
        Ok(())
    }

    /// One binary `HomeRequest`, under the grants the JSON requests need.
    pub(super) fn home(&self, request: HomeRequest) -> Result<HomeResponse, Refusal> {
        let Some(request) = request.request else {
            return Err(refuse("invalid_request", "empty home request"));
        };
        let cap = match request {
            Request::ProjectsOverview(_) | Request::AttentionRows(_) => Capability::Read,
            Request::AttentionDismiss(_) => Capability::Approve,
        };
        if !self.caps.contains(&cap) {
            return Err(refuse(
                "forbidden",
                format!("this request requires the {} capability", cap.as_str()),
            ));
        }
        let app = &self.app;
        let response = match request {
            Request::ProjectsOverview(r) => {
                crate::overview::overview(app, &r.project_ids).map(Response::ProjectsOverview)
            }
            Request::AttentionRows(r) => {
                crate::overview::attention_rows(app, &r.project_id).map(Response::AttentionRows)
            }
            Request::AttentionDismiss(r) => {
                crate::overview::dismiss(&r.id).map(Response::AttentionDismissed)
            }
        };
        response
            .map(|response| HomeResponse {
                response: Some(response),
            })
            .map_err(|e| match crate::decisions::error_code(&e) {
                Some(code) => refuse(code, e.to_string()),
                None => Refusal::from(e),
            })
    }
}
