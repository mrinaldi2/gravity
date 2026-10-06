//! Requests answered off the connection's task, a panic in their handler
//! contained (H-170). A spawned handler that panicked used to drop its
//! answer: the client waited on that request until it gave up. Now it
//! answers `internal` under the request's own `req_id`.

use std::future::Future;

use serde_json::{json, Value};

use super::{binary, Conn};
use crate::contain;

impl Conn {
    /// Sends the reply `work(req_id)` builds, or `internal` if it panics.
    pub(super) fn spawn_reply<W>(
        &self,
        req_id: &Value,
        work: impl FnOnce(Value) -> W + Send + 'static,
    ) where
        W: Future<Output = Value> + Send + 'static,
    {
        let (out, req_id, kind) = (self.out.clone(), req_id.clone(), self.kind.clone());
        tokio::spawn(async move {
            let answer_to = req_id.clone();
            let work = async move { work(answer_to).await };
            let reply = contain::run_async(&kind, work, || internal(&req_id, &kind)).await;
            let _ = out.send(reply);
        });
    }

    /// [`Conn::spawn_reply`] for a handler that blocks.
    pub(super) fn spawn_blocking_reply(
        &self,
        req_id: &Value,
        work: impl FnOnce(Value) -> Value + Send + 'static,
    ) {
        let (out, req_id, kind) = (self.out.clone(), req_id.clone(), self.kind.clone());
        tokio::task::spawn_blocking(move || {
            let answer_to = req_id.clone();
            let reply = contain::run(&kind, || work(answer_to), || internal(&req_id, &kind));
            let _ = out.send(reply);
        });
    }

    /// Sends the binary frame `work` builds, or an `internal` error under
    /// `req_id` if it panics.
    pub(super) fn spawn_frame(
        &self,
        req_id: u64,
        kind: &'static str,
        work: impl Future<Output = Vec<u8>> + Send + 'static,
    ) {
        let bin = self.bin.clone();
        tokio::spawn(async move {
            let frame = contain::run_async(kind, work, || {
                binary::error(req_id, "internal", contain::internal_message(kind))
            })
            .await;
            let _ = bin.send(frame);
        });
    }
}

/// Work this request started that answers nothing, its panic logged under
/// the request's kind instead of tokio's anonymous report.
pub(super) fn spawn_quiet(kind: String, work: impl Future<Output = ()> + Send + 'static) {
    tokio::spawn(async move { contain::run_async(&kind, work, || ()).await });
}

/// The answer to a request whose handler panicked.
pub(super) fn internal(req_id: &Value, kind: &str) -> Value {
    json!({
        "type": "error", "req_id": req_id, "code": "internal",
        "message": contain::internal_message(kind)
    })
}
