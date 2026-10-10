//! Binary WebSocket frames: one protobuf `Envelope` each (ADR-001 §1). Typed
//! surfaces ride here while text frames stay the JSON protocol v2. A request
//! is answered under its own `req_id`; pushes carry `req_id` 0. Anything a
//! client may not send is answered with an `Error` naming why.

use bus::contract::board::{BoardPush, BoardRequest, BoardResponse};
use bus::contract::home::{HomeRequest, HomeResponse};
use bus::contract::wire::{envelope::Body, Envelope, Error};
use prost::Message;

/// A request the daemon serves, or the error frame to answer instead.
pub(super) enum Frame {
    Board(u64, BoardRequest),
    Home(u64, HomeRequest),
    Refused(Vec<u8>),
}

impl Frame {
    /// The request's id, for an answer when its handler fails.
    pub(super) fn req_id(&self) -> u64 {
        match self {
            Frame::Board(req_id, _) | Frame::Home(req_id, _) => *req_id,
            Frame::Refused(_) => 0,
        }
    }

    /// What is asked, for the log: `board:ItemGet`, `home:ProjectsOverview`.
    pub(super) fn kind(&self) -> String {
        match self {
            Frame::Board(_, r) => kind("board", r.request.as_ref()),
            Frame::Home(_, r) => kind("home", r.request.as_ref()),
            Frame::Refused(_) => "refused".to_string(),
        }
    }
}

/// A request's surface and its variant's name, taken from its `Debug` form.
fn kind(surface: &str, request: Option<&impl std::fmt::Debug>) -> String {
    let variant = request.map(|r| format!("{r:?}")).unwrap_or_default();
    let name = variant.split(['(', ' ', '{']).next().unwrap_or("");
    format!("{surface}:{name}")
}

pub(super) fn decode(frame: &[u8]) -> Frame {
    let (req_id, message) = match Envelope::decode(frame) {
        Err(e) => (0, format!("not a protobuf Envelope: {e}")),
        Ok(Envelope {
            req_id,
            body: Some(Body::BoardRequest(request)),
        }) => return Frame::Board(req_id, request),
        Ok(Envelope {
            req_id,
            body: Some(Body::HomeRequest(request)),
        }) => return Frame::Home(req_id, request),
        // The contract is in (H-265); the daemon serves it from H-273.
        Ok(Envelope {
            req_id,
            body: Some(Body::PrRequest(_)),
        }) => {
            return Frame::Refused(error(
                req_id,
                "unsupported",
                "pull requests aren't served by this daemon yet".to_string(),
            ))
        }
        Ok(Envelope { req_id, body: None }) => (req_id, "empty envelope".to_string()),
        Ok(Envelope {
            req_id,
            body:
                Some(
                    Body::Error(_)
                    | Body::BoardResponse(_)
                    | Body::BoardPush(_)
                    | Body::HomeResponse(_)
                    | Body::PrResponse(_)
                    | Body::PrPush(_),
                ),
        }) => (
            req_id,
            "clients send requests, not errors, responses or pushes".to_string(),
        ),
    };
    Frame::Refused(error(req_id, "invalid_request", message))
}

pub(super) fn response(req_id: u64, response: BoardResponse) -> Vec<u8> {
    encode(req_id, Body::BoardResponse(response))
}

pub(super) fn home_response(req_id: u64, response: HomeResponse) -> Vec<u8> {
    encode(req_id, Body::HomeResponse(response))
}

pub(super) fn push(push: BoardPush) -> Vec<u8> {
    encode(0, Body::BoardPush(push))
}

pub(super) fn error(req_id: u64, code: &str, message: String) -> Vec<u8> {
    encode(
        req_id,
        Body::Error(Error {
            code: code.to_string(),
            message,
        }),
    )
}

fn encode(req_id: u64, body: Body) -> Vec<u8> {
    Envelope {
        req_id,
        body: Some(body),
    }
    .encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_of(frame: Frame) -> (u64, String) {
        let Frame::Refused(bytes) = frame else {
            panic!("expected a refusal");
        };
        let envelope = Envelope::decode(bytes.as_slice()).expect("the reply decodes");
        match envelope.body {
            Some(Body::Error(e)) => (envelope.req_id, e.code),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn garbage_is_refused_without_a_request_id() {
        assert_eq!(
            error_of(decode(&[0xff, 0xff, 0xff])),
            (0, "invalid_request".into())
        );
    }

    #[test]
    fn an_envelope_is_answered_under_its_own_request_id() {
        let request = Envelope {
            req_id: 42,
            body: None,
        }
        .encode_to_vec();
        assert_eq!(error_of(decode(&request)), (42, "invalid_request".into()));
    }

    #[test]
    fn a_client_may_not_send_a_push() {
        let request = Envelope {
            req_id: 7,
            body: Some(Body::BoardPush(BoardPush::default())),
        }
        .encode_to_vec();
        assert_eq!(error_of(decode(&request)), (7, "invalid_request".into()));
    }

    #[test]
    fn a_home_request_is_served() {
        let request = Envelope {
            req_id: 11,
            body: Some(Body::HomeRequest(HomeRequest::default())),
        }
        .encode_to_vec();
        assert!(matches!(decode(&request), Frame::Home(11, _)));
    }

    #[test]
    fn a_pr_request_is_unsupported_until_the_daemon_serves_it() {
        let request = Envelope {
            req_id: 13,
            body: Some(Body::PrRequest(bus::contract::pr::PrRequest::default())),
        }
        .encode_to_vec();
        assert_eq!(error_of(decode(&request)), (13, "unsupported".into()));
    }

    #[test]
    fn a_board_request_is_served() {
        let request = Envelope {
            req_id: 9,
            body: Some(Body::BoardRequest(BoardRequest::default())),
        }
        .encode_to_vec();
        assert!(matches!(decode(&request), Frame::Board(9, _)));
    }
}
