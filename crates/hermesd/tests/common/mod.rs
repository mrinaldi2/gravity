//! Shared harness for the end-to-end tests: a real daemon instance backed by
//! the deterministic runtime double, plus a small WebSocket client.
//!
//! Each integration binary compiles the whole harness but uses only part of it.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use hermesd::app::AppState;
use hermesd::config::{Config, RuntimeKind};
use hermesd::db::Db;
use hermesd::runtime::RuntimeAdapter;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message as WsMsg;

pub mod board;
pub mod deployed_via;
pub mod devtools;
pub mod grants;
pub mod ios_targets;
pub mod peer_board;
pub mod peers;
pub mod pr_wire;
pub mod proxy;
pub mod prs;
pub mod release_phone;
pub mod releases;
pub mod repo;
pub mod supersede;
pub mod tasks;
pub mod team;

/// How long a test waits for a frame it expects.
pub const FRAME_WAIT: Duration = Duration::from_secs(5);

/// How long a test waits for a frame while it sets up: the hello, projects
/// and bots. The first run after a build is cold, and every test binary
/// starts its daemons at once, so a bot's workspace can take far longer to
/// provision than [`FRAME_WAIT`] on a slow disk or under a virus scan.
/// `HERMES_TEST_SETUP_WAIT_SECS` overrides it.
pub fn setup_wait() -> Duration {
    std::env::var("HERMES_TEST_SETUP_WAIT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map_or(Duration::from_secs(60), Duration::from_secs)
}

pub struct TestDaemon {
    pub app: Arc<AppState>,
    pub addr: SocketAddr,
    pub _home: tempfile::TempDir,
}

pub async fn spawn_daemon() -> TestDaemon {
    spawn_daemon_with(|_| {}).await
}

/// Spawn a daemon with the config tweaked before anything reads it — the
/// supervisor keeps its own clone, so limits must be set up front.
pub async fn spawn_daemon_with(tweak: impl FnOnce(&mut Config)) -> TestDaemon {
    spawn_daemon_on(None, tweak).await
}

/// [`spawn_daemon_with`] around a given runtime instead of the double.
pub async fn spawn_daemon_on(
    adapter: Option<Arc<dyn RuntimeAdapter>>,
    tweak: impl FnOnce(&mut Config),
) -> TestDaemon {
    let home = tempfile::tempdir().expect("tempdir");
    let mut cfg = Config {
        home: home.path().to_path_buf(),
        runtime: RuntimeKind::Double,
        ..Config::default()
    };
    cfg.delivery.poll_interval_ms = 50;
    cfg.scheduler.tick_interval_ms = 100;
    // No tool probe or check routing behind a test's back (H-283).
    cfg.checks.probe_interval_secs = 0;
    cfg.checks.dispatch_interval_secs = 0;
    // The test binary isn't hermesd: checks run through the real runner.
    cfg.checks.runner = Some(env!("CARGO_BIN_EXE_hermesd").into());
    // A shared computer's free disk is no test's business; AC2 sets its own.
    cfg.checks.disk_floor_gb = 0;
    tweak(&mut cfg);

    // SafeGit's home is the process's, and the first daemon names it: name
    // one that outlives every test here, not a daemon's tempdir that goes
    // when its test ends while others still run git.
    static GIT_HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    hermesd::safe_git::set_home(GIT_HOME.get_or_init(|| tempfile::tempdir().unwrap().keep()));
    let db = Db::open(&cfg.db_path()).expect("db");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    cfg.port = addr.port();
    let app = match adapter {
        Some(adapter) => AppState::with_adapter(cfg, db, adapter),
        None => AppState::new(cfg, db),
    }
    .expect("app");
    hermesd::server::spawn_workers(&app);
    let router = hermesd::server::router(app.clone());
    tokio::spawn(async move {
        let service = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
        let _ = axum::serve(listener, service).await;
    });
    TestDaemon {
        app,
        addr,
        _home: home,
    }
}

pub struct WsClient {
    pub tx: futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        WsMsg,
    >,
    pub rx: futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    pub next_req: u64,
}

impl WsClient {
    /// Connects as the desktop app: on its one-time ticket, showing
    /// permission and terminal cards. Only a ticket or a device types into a
    /// bot or answers its prompts (H-195 D5).
    pub async fn connect(d: &TestDaemon) -> Self {
        Self::connect_as(d, &d.app.owner.mint()).await
    }

    /// Connects on the owner token file, as a bot of the same user could.
    pub async fn connect_owner_token(d: &TestDaemon) -> Self {
        Self::connect_as(d, d.app.secrets.client_token()).await
    }

    /// Connects as the owner on a ticket, saying it supports these hello
    /// features only.
    pub async fn connect_with_features(d: &TestDaemon, features: &[&str]) -> Self {
        Self::connect_with(d, &d.app.owner.mint(), features).await
    }

    /// Connects on `token` as the desktop app does: the client token, a
    /// ticket or a device's token.
    pub async fn connect_as(d: &TestDaemon, token: &str) -> Self {
        Self::connect_with(d, token, &["permission_cards", "terminal_card"]).await
    }

    /// Connects on `token`, saying it supports these hello features only.
    pub async fn connect_with(d: &TestDaemon, token: &str, features: &[&str]) -> Self {
        Self::connect_hello(d, token, json!({ "features": features })).await
    }

    /// Connects on `token` with these fields added to the hello.
    pub async fn connect_hello(d: &TestDaemon, token: &str, extra: Value) -> Self {
        let url = format!("ws://{}/ws", d.addr);
        let (socket, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("ws connect");
        let (tx, rx) = socket.split();
        let mut c = Self {
            tx,
            rx,
            next_req: 1,
        };
        // The hello is the first frame a fresh daemon answers: once it is
        // back, the daemon is serving.
        let mut hello = json!({
            "type": "hello",
            "protocol_version": 2,
            "token": token,
            "client": "test/0",
        });
        if let (Some(hello), Some(extra)) = (hello.as_object_mut(), extra.as_object()) {
            hello.extend(extra.clone());
        }
        let reply = c.request_within(setup_wait(), hello).await;
        assert_eq!(reply["type"], "hello_ok", "handshake failed: {reply}");
        c
    }

    pub async fn send(&mut self, mut req: Value) -> String {
        let req_id = self.next_req.to_string();
        self.next_req += 1;
        req["req_id"] = json!(req_id);
        self.tx
            .send(WsMsg::Text(req.to_string()))
            .await
            .expect("ws send");
        req_id
    }

    /// Send a request and wait for the frame carrying its req_id, buffering
    /// nothing (pushes are skipped).
    pub async fn request(&mut self, req: Value) -> Value {
        self.request_within(FRAME_WAIT, req).await
    }

    /// `request` with a deadline of its own, such as [`setup_wait`].
    pub async fn request_within(&mut self, within: Duration, req: Value) -> Value {
        let req_id = self.send(req).await;
        self.wait_for_within(within, |v| v["req_id"] == json!(req_id.clone()))
            .await
    }

    pub async fn wait_for(&mut self, pred: impl Fn(&Value) -> bool) -> Value {
        self.wait_for_within(FRAME_WAIT, pred).await
    }

    /// `wait_for` with a deadline of its own, for a frame behind heavy traffic.
    pub async fn wait_for_within(
        &mut self,
        within: Duration,
        pred: impl Fn(&Value) -> bool,
    ) -> Value {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let frame = tokio::time::timeout_at(deadline, self.rx.next())
                .await
                .expect("timed out waiting for frame")
                .expect("stream ended")
                .expect("ws error");
            if let WsMsg::Text(text) = frame {
                let v: Value = serde_json::from_str(&text).expect("json frame");
                if pred(&v) {
                    return v;
                }
            }
        }
    }
}

pub async fn create_bot(c: &mut WsClient, project_id: &str, name: &str) -> Value {
    let bot = c
        .request_within(
            setup_wait(),
            json!({
                "type": "create_bot", "project_id": project_id, "name": name,
                "description": format!("{name} bot"), "instructions": "be helpful"
            }),
        )
        .await;
    assert_eq!(bot["type"], "bot", "{bot}");
    bot["bot"].clone()
}

pub async fn raw_hello(d: &TestDaemon, token: &str) -> Value {
    let url = format!("ws://{}/ws", d.addr);
    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect");
    let (mut tx, mut rx) = socket.split();
    tx.send(WsMsg::Text(
        json!({"type": "hello", "req_id": "1", "protocol_version": 2, "token": token}).to_string(),
    ))
    .await
    .expect("send hello");
    loop {
        let frame = rx.next().await.expect("frame").expect("ws ok");
        if let WsMsg::Text(text) = frame {
            return serde_json::from_str(&text).expect("json");
        }
    }
}

pub fn token_str(created: &Value) -> &str {
    created["token"].as_str().expect("token")
}

/// Minimal MCP client speaking the daemon's JSON-RPC subset as one bot.
///
/// Identity comes from the bearer token, exactly as it does for a real bot, so
/// these tests exercise the same authorisation path production does.
pub struct McpClient {
    url: String,
    token: String,
    http: reqwest::Client,
    next_id: u64,
}

impl McpClient {
    pub fn new(d: &TestDaemon, token: &str) -> Self {
        Self {
            url: format!("http://{}/mcp", d.addr),
            token: token.to_string(),
            http: reqwest::Client::new(),
            next_id: 1,
        }
    }

    /// Call a tool and return its decoded JSON payload, asserting success.
    pub async fn call(&mut self, name: &str, args: Value) -> Value {
        let raw = self.call_raw(name, args).await;
        assert_ne!(raw["isError"], json!(true), "tool {name} failed: {raw}");
        let text = raw["content"][0]["text"].as_str().expect("text content");
        serde_json::from_str(text).expect("tool payload is json")
    }

    /// Call a tool and return the raw MCP result, errors included.
    pub async fn call_raw(&mut self, name: &str, args: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": args }
        });
        let reply: Value = self
            .http
            .post(&self.url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .expect("mcp request")
            .json()
            .await
            .expect("mcp json");
        reply["result"].clone()
    }

    /// The advertised tool list.
    pub async fn tools(&mut self) -> Value {
        let body = json!({ "jsonrpc": "2.0", "id": 99, "method": "tools/list" });
        let reply: Value = self
            .http
            .post(&self.url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .expect("mcp request")
            .json()
            .await
            .expect("mcp json");
        reply["result"].clone()
    }

    /// Whether this client's token still authenticates at all.
    pub async fn is_authorized(&self) -> bool {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
        let status = self
            .http
            .post(&self.url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .expect("mcp request")
            .status();
        status.is_success()
    }
}

/// Everything the bot's terminal has shown so far. The runtime double opens
/// with the flags it was spawned with, so this is also how a test sees what
/// the daemon asked the runtime for.
pub fn terminal(d: &TestDaemon, bot_id: &str) -> String {
    let term = d.app.supervisor.term(bot_id).expect("terminal buffer");
    let bytes: Vec<u8> = term
        .replay_after(0)
        .frames
        .into_iter()
        .flat_map(|f| f.data)
        .collect();
    String::from_utf8_lossy(&bytes).to_string()
}
