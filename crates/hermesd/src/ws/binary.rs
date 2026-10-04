//! Binary WebSocket frames: one protobuf `Envelope` each (ADR-001 §1). Typed
//! surfaces ride here while text frames stay the JSON protocol v2. No surface
//! is served yet (the board's requests arrive in B4), so every envelope is
//! answered with an error naming why.

use bus::contract::wire::{envelope::Body, Envelope, Error};
use prost::Message;

/// The reply to one binary frame.
pub(super) fn reply(frame: &[u8]) -> Vec<u8> {
    let (req_id, code, message) = match Envelope::decode(frame) {
        Err(e) => (0, "invalid_request", format!("not a protobuf Envelope: {e}")),
        Ok(Envelope { req_id, body: None }) => (req_id, "invalid_request", "empty envelope".to_string()),
        Ok(Envelope { req_id, body: Some(Body::Error(_)) }) => {
            (req_id, "invalid_request", "clients do not send errors".to_string())
        }
    };
    Envelope {
        req_id,
        body: Some(Body::Error(Error {
            code: code.to_string(),
            message,
        })),
    }
    .encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_of(bytes: &[u8]) -> (u64, String) {
        let envelope = Envelope::decode(bytes).expect("the reply decodes");
        match envelope.body {
            Some(Body::Error(e)) => (envelope.req_id, e.code),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn garbage_is_refused_without_a_request_id() {
        assert_eq!(error_of(&reply(&[0xff, 0xff, 0xff])), (0, "invalid_request".into()));
    }

    #[test]
    fn an_envelope_is_answered_under_its_own_request_id() {
        let request = Envelope { req_id: 42, body: None }.encode_to_vec();
        assert_eq!(error_of(&reply(&request)), (42, "invalid_request".into()));
    }
}
