//! Test-only handlers that panic, for the containment tests (H-170). None
//! of this is compiled outside `cargo test`.

use bus::contract::board::board_request::Request;
use serde_json::Value;

use super::binary::Frame;
use super::Conn;

/// A binary `BoardGet` for this project panics in its handler.
pub(super) const PANIC_PROJECT: &str = "test-panic";
/// A binary `BoardGet` for this project panics in the task answering it.
pub(super) const PANIC_LATER_PROJECT: &str = "test-panic-later";
/// A JSON request of this type panics in the connection's own task, outside
/// any handler's containment.
pub(super) const PANIC_CONNECTION: &str = "test_panic_connection";
/// The project a mid-write panic tried to create.
pub(super) const UNWRITTEN_PROJECT: &str = "never written";

async fn panics<T>() -> T {
    panic!("a test handler panicked off the connection's task")
}

impl Conn {
    /// Serves a JSON probe, or `None` when `kind` is not one.
    pub(super) fn probe(&mut self, kind: &str, req_id: &Value) -> Option<anyhow::Result<()>> {
        match kind {
            "test_panic" => panic!("a test handler panicked"),
            "test_panic_later" => self.answer_later(req_id, panics()),
            "test_panic_blocking" => {
                self.spawn_blocking_reply(req_id, |_| panic!("a test blocking handler panicked"))
            }
            "test_panic_mid_write" => self.app.db.panic_mid_write(UNWRITTEN_PROJECT),
            _ => return None,
        }
        Some(Ok(()))
    }

    /// Serves a binary probe; false when `frame` is not one.
    pub(super) fn probe_binary(&mut self, frame: &Frame) -> bool {
        let Frame::Board(req_id, request) = frame else {
            return false;
        };
        let Some(Request::BoardGet(get)) = &request.request else {
            return false;
        };
        match get.project_id.as_str() {
            PANIC_PROJECT => panic!("a test binary handler panicked"),
            PANIC_LATER_PROJECT => self.spawn_frame(*req_id, "board:BoardGet", panics()),
            _ => return false,
        }
        true
    }
}
