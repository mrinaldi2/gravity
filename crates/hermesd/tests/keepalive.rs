//! A dead link is noticed: the daemon pings every client and closes a
//! connection that has sent nothing for a minute, ending everything it was
//! doing. A phone that drives into a tunnel no longer leaves its connection,
//! and the browser screencast it watched, running until the OS gives up.

mod common;

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;

use common::devtools::{fake_devtools, start_browser};
use common::peers::{project, wait_until};
use common::*;
use futures::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as WsMsg;

#[tokio::test]
async fn a_silent_client_is_dropped_and_its_watch_ends() {
    let d = spawn_daemon_with(|cfg| cfg.user_home = cfg.home.join("user")).await;
    let mut c = WsClient::connect(&d).await;
    let pid = project(&mut c, "web").await;
    let bot = create_bot(&mut c, &pid, "surfer").await;
    let id = bot["id"].as_str().expect("id").to_string();
    let port = fake_devtools("Example", Arc::new(AtomicUsize::new(0))).await;
    start_browser(&d, &id, port);
    c.request(json!({"type": "watch_browser", "bot_id": id}))
        .await;
    c.wait_for(|v| v["type"] == "browser_frame").await;
    assert_eq!(d.app.browsers.live(), 1);

    // The phone loses its link: nothing more comes from it, not even a pong.
    tokio::time::pause();
    tokio::time::sleep(Duration::from_secs(50)).await;
    assert_eq!(d.app.browsers.live(), 1, "dropped inside the idle window");
    tokio::time::sleep(Duration::from_secs(15)).await;
    let app = d.app.clone();
    wait_until("the watch ends with the connection", || {
        app.browsers.live() == 0
    })
    .await;

    // What the daemon sent meanwhile: its pings, then the end.
    let mut pings = 0;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), c.rx.next())
            .await
            .expect("the connection is still open");
        match frame {
            Some(Ok(WsMsg::Ping(_))) => pings += 1,
            Some(Ok(WsMsg::Close(_)) | Err(_)) | None => break,
            Some(Ok(_)) => {}
        }
    }
    assert!(pings >= 2, "{pings} pings in a minute");
}

#[tokio::test]
async fn a_client_that_pings_stays_connected_and_gets_its_pongs() {
    let d = spawn_daemon().await;
    let mut c = WsClient::connect(&d).await;

    // Ninety seconds of nothing but the phone's own pings.
    tokio::time::pause();
    for _ in 0..9 {
        tokio::time::sleep(Duration::from_secs(10)).await;
        c.tx.send(WsMsg::Ping(b"phone".to_vec()))
            .await
            .expect("ping");
    }

    let pong = loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), c.rx.next())
            .await
            .expect("a pong");
        match frame {
            Some(Ok(WsMsg::Pong(data))) => break data,
            Some(Ok(_)) => {}
            other => panic!("connection ended: {other:?}"),
        }
    };
    assert_eq!(pong, b"phone");
    let reply = c.request(json!({"type": "list_projects"})).await;
    assert_eq!(reply["type"], "projects", "{reply}");
}
