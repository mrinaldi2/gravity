//! The one task that writes to a connection's socket.

use std::time::Duration;

use axum::extract::ws::Message as WsMessage;
use futures::{Sink, SinkExt};
use serde_json::Value;
use tokio::sync::{mpsc, watch};

/// Writes a connection's outbound traffic until `out` closes or the socket
/// fails, then closes the socket: replies and pushes in order, a ping every `ping_every`, and browser
/// frames from the newest-wins slot only when nothing else is waiting. At
/// most one frame is ever being written, so a slow link skips frames and the
/// rest of the traffic waits behind one frame at most. Binary protobuf
/// envelopes (`binary`) are written as they come, like replies.
pub(super) async fn write_out<S>(
    mut sink: S,
    mut out: mpsc::UnboundedReceiver<Value>,
    mut binary: mpsc::UnboundedReceiver<Vec<u8>>,
    mut frames: watch::Receiver<Option<Value>>,
    ping_every: Duration,
) where
    S: Sink<WsMessage> + Unpin,
{
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + ping_every, ping_every);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut watching = true;
    loop {
        let message = tokio::select! {
            biased;
            next = out.recv() => match next {
                Some(v) => WsMessage::Text(v.to_string()),
                None => break,
            },
            Some(bytes) = binary.recv() => WsMessage::Binary(bytes),
            _ = ping.tick() => WsMessage::Ping(Vec::new()),
            changed = frames.changed(), if watching => {
                if changed.is_err() {
                    watching = false;
                    continue;
                }
                let Some(text) = frames.borrow_and_update().as_ref().map(Value::to_string) else {
                    continue;
                };
                WsMessage::Text(text)
            }
        };
        // `send` flushes, which also lets out any pong queued for the client.
        if sink.send(message).await.is_err() {
            break;
        }
    }
    // A close frame, so the client sees the connection end at once and
    // reconnects (H-170).
    let _ = sink.close().await;
}

#[cfg(test)]
mod tests {
    use futures::channel::mpsc as link;
    use futures::StreamExt;
    use serde_json::json;

    use super::*;
    use crate::browser::view::Viewer;

    const NEVER: Duration = Duration::from_secs(3600);

    async fn next(link: &mut link::Receiver<WsMessage>) -> Value {
        match link.next().await {
            Some(WsMessage::Text(text)) => serde_json::from_str(&text).expect("json"),
            other => panic!("expected a text frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_stalled_link_skips_frames_and_lets_a_reply_through() {
        // A link with room for one message that nobody reads: the first
        // write never completes until the test reads it.
        let (sink, mut link) = link::channel::<WsMessage>(0);
        let (out, out_rx) = mpsc::unbounded_channel();
        let (viewer, frames) = Viewer::new(out.clone());
        let writer = tokio::spawn(write_out(
            sink,
            out_rx,
            mpsc::unbounded_channel().1,
            frames,
            NEVER,
        ));

        for n in 0..1000 {
            assert!(viewer.frame(json!({ "type": "browser_frame", "n": n })));
            tokio::task::yield_now().await;
        }
        out.send(json!({ "type": "ok", "req_id": "after" }))
            .expect("send");
        tokio::task::yield_now().await;

        // The link comes back: the frame that was being written, the reply,
        // then only the newest frame.
        let mut before_reply = 0;
        loop {
            let message = next(&mut link).await;
            if message["req_id"] == "after" {
                break;
            }
            before_reply += 1;
        }
        assert!(
            before_reply <= 1,
            "{before_reply} frames went before the reply"
        );
        assert_eq!(next(&mut link).await["n"], 999);
        tokio::task::yield_now().await;
        assert!(
            link.try_recv().is_err(),
            "more than the newest frame was queued"
        );

        drop((out, viewer));
        writer.await.expect("writer");
    }

    #[tokio::test]
    async fn a_cleared_slot_sends_nothing() {
        let (sink, mut link) = link::channel::<WsMessage>(8);
        let (out, out_rx) = mpsc::unbounded_channel();
        let (viewer, frames) = Viewer::new(out.clone());
        let writer = tokio::spawn(write_out(
            sink,
            out_rx,
            mpsc::unbounded_channel().1,
            frames,
            NEVER,
        ));

        viewer.frame(json!({ "type": "browser_frame" }));
        viewer.clear();
        out.send(json!({ "type": "ok" })).expect("send");
        assert_eq!(next(&mut link).await["type"], "ok");
        tokio::task::yield_now().await;
        assert!(link.try_recv().is_err(), "a cleared frame was sent");

        drop((out, viewer));
        writer.await.expect("writer");
    }

    #[tokio::test(start_paused = true)]
    async fn the_writer_pings_on_schedule_and_stops_when_the_connection_ends() {
        let (sink, mut link) = link::channel::<WsMessage>(8);
        let (out, out_rx) = mpsc::unbounded_channel();
        let (viewer, frames) = Viewer::new(out.clone());
        let writer = tokio::spawn(write_out(
            sink,
            out_rx,
            mpsc::unbounded_channel().1,
            frames,
            Duration::from_secs(20),
        ));

        tokio::time::sleep(Duration::from_secs(19)).await;
        assert!(link.try_recv().is_err(), "pinged early");
        tokio::time::sleep(Duration::from_secs(42)).await;
        let mut pings = 0;
        while let Ok(message) = link.try_recv() {
            assert!(matches!(message, WsMessage::Ping(_)), "{message:?}");
            pings += 1;
        }
        assert_eq!(pings, 3);

        drop((out, viewer));
        writer.await.expect("writer");
    }
}
