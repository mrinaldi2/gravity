//! Answering one binary frame (ADR-001): the board's, the projects home's
//! and the PRs' typed requests, each under its own `req_id`.

use bus::contract::board as c;

use super::binary::{self, Frame};
use super::Conn;

impl Conn {
    /// Answer one binary frame.
    pub(super) fn binary_frame(&mut self, frame: Frame) {
        let reply = match frame {
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
            Frame::Home(req_id, request) => match self.home(req_id, request) {
                Ok(Some(response)) => binary::home_response(req_id, response),
                Ok(None) => return,
                Err(r) => binary::error(req_id, r.code, r.message),
            },
            Frame::Pr(req_id, request) => match self.pr_frame(req_id, request) {
                Ok(()) => return,
                Err(r) => binary::error(req_id, r.code, r.message),
            },
        };
        let _ = self.bin.send(reply);
    }
}
