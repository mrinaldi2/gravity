//! A client for the PR surface's binary frames (H-273).

use std::time::Duration;

use bus::contract::pr::{self as p, pr_request::Request, pr_response::Response};
use bus::contract::wire::{envelope::Body, Envelope};
use prost::Message;

use super::board::{next_envelope, send_raw};
use super::WsClient;

/// Send one PR request and wait for the envelope answering it, keeping the
/// PR pushes that came first in `pushes`.
pub async fn call_keeping(c: &mut WsClient, request: Request, pushes: &mut Vec<p::PrPush>) -> Body {
    let req_id = c.next_req;
    c.next_req += 1;
    let envelope = Envelope {
        req_id,
        body: Some(Body::PrRequest(p::PrRequest {
            request: Some(request),
        })),
    };
    send_raw(c, envelope.encode_to_vec()).await;
    loop {
        let envelope = next_envelope(c, Duration::from_secs(20))
            .await
            .expect("a reply");
        if envelope.req_id == req_id {
            return envelope.body.expect("a body");
        }
        if let Some(Body::PrPush(push)) = envelope.body {
            pushes.push(push);
        }
    }
}

pub async fn call(c: &mut WsClient, request: Request) -> Body {
    call_keeping(c, request, &mut Vec::new()).await
}

pub fn response(body: Body) -> Response {
    match body {
        Body::PrResponse(r) => r.response.expect("a response"),
        other => panic!("expected a PR response, got {other:?}"),
    }
}

pub fn error(body: Body) -> (String, String) {
    match body {
        Body::Error(e) => (e.code, e.message),
        other => panic!("expected an error, got {other:?}"),
    }
}

pub async fn list(c: &mut WsClient, project: &str, states: &[p::PrState]) -> Vec<p::PullRequest> {
    let request = Request::PrList(p::PrListRequest {
        project_id: project.into(),
        states: states.iter().map(|s| *s as i32).collect(),
    });
    match response(call(c, request).await) {
        Response::PrList(l) => l.prs,
        other => panic!("expected a list, got {other:?}"),
    }
}

pub fn get(project: &str, number: u32) -> Request {
    Request::PrGet(p::PrGetRequest {
        project_id: project.into(),
        number,
    })
}

pub async fn pr(c: &mut WsClient, project: &str, number: u32) -> p::PullRequest {
    match response(call(c, get(project, number)).await) {
        Response::Pr(pr) => pr,
        other => panic!("expected a PR, got {other:?}"),
    }
}

pub fn diff(project: &str, from: Option<&str>, to: Option<&str>, path: Option<&str>) -> Request {
    Request::PrDiff(p::PrDiffRequest {
        project_id: project.into(),
        number: 1,
        from_sha: from.map(str::to_string),
        to_sha: to.map(str::to_string),
        path: path.map(str::to_string),
    })
}

pub fn rerun(project: &str, sha: &str, name: &str) -> Request {
    Request::CheckRerun(p::CheckRerunRequest {
        project_id: project.into(),
        sha: sha.into(),
        name: name.into(),
    })
}

/// The next PR push within `within`, skipping other frames; `None` if none.
pub async fn next_push(c: &mut WsClient, within: Duration) -> Option<p::pr_push::Push> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let envelope = next_envelope(c, left).await?;
        if let Some(Body::PrPush(push)) = envelope.body {
            assert_eq!(envelope.req_id, 0, "pushes carry req_id 0");
            return push.push;
        }
    }
}

/// Waits for `want` among the PR pushes, within 10 s.
pub async fn expect_push(c: &mut WsClient, want: p::pr_push::Push) {
    let mut seen = Vec::new();
    while let Some(push) = next_push(c, Duration::from_secs(10)).await {
        if push == want {
            return;
        }
        seen.push(push);
    }
    panic!("no {want:?} push; saw {seen:?}");
}
