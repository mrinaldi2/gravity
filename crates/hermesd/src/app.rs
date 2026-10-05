//! Shared daemon state wiring.

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crate::config::{Config, RuntimeKind};
use crate::db::Db;
use crate::events::Events;
use crate::overrides::{AutoCompactOverride, AUTO_COMPACT_META_KEY};
use crate::runtime::double::DoubleAdapter;
use crate::runtime::mixed::MixedAdapter;
use crate::runtime::RuntimeAdapter;
use crate::secrets::Secrets;
use crate::supervisor::Supervisor;

pub const PROTOCOL_VERSION: u32 = 2;
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Features this daemon serves, advertised in `hello_ok`. Informational: the
/// client uses it to hide a panel it cannot fill, while authorisation stays
/// with the connection's grants.
pub const CAPABILITIES: &[&str] = &[
    "terminal_attach",
    "search",
    "routines",
    "devices",
    "bot_self_management",
    "config",
    "decisions",
    "bot_runtime",
    "chat",
    "permissions",
    "peer_chat",
    "linked_projects",
    "bot_browser",
    "agent_conversations",
    "peer_terminal",
    "peer_browser",
    "browser_input",
    "bot_commands",
    "restart_bot",
    "workers",
    "permission_profiles",
];

pub struct AppState {
    pub cfg: Config,
    pub db: Db,
    pub events: Events,
    pub secrets: Arc<Secrets>,
    pub supervisor: Supervisor,
    /// Runtime override for `cfg.auto_compact_window`, shared with the
    /// supervisor and persisted in the `meta` table.
    pub auto_compact: AutoCompactOverride,
    /// Live links to peer daemons, shared by the delivery worker (forwarding
    /// to linked bots) and the control plane (pairing and linking).
    pub peers: crate::peer::PeerHub,
    /// Bots' conversations read from their transcripts, for the chat pane.
    pub chat: crate::chat::ChatStore,
    /// Permission prompts waiting on the owner's answer.
    pub approvals: crate::approval::Approvals,
    /// Bot browsers being watched, one shared stream each.
    pub browsers: crate::browser::streams::BrowserStreams,
    /// The temporary-worker queue's wake-up and placement lock.
    pub workers: crate::workers::Workers,
    /// Board changes, numbered per project, for the clients watching them.
    pub board: crate::board::feed::BoardFeed,
    /// Boards whose home is a peer, as last seen (B9).
    pub board_mirror: crate::board::mirror::BoardMirror,
    /// Bots that used a bearer token, and when (H-044 phase 1).
    pub bearers: crate::bus_auth::BearerLog,
    /// The owner's one-time tickets and the app check behind them (H-044 T4).
    pub owner: crate::bus_auth::owner::Owner,
    pub started_at: Instant,
    /// Wall-clock start, kept alongside the monotonic `started_at` purely so
    /// [`AppState::stale_build`] can compare it against a file mtime.
    started_wall: SystemTime,
}

impl AppState {
    pub fn new(cfg: Config, db: Db) -> anyhow::Result<Arc<Self>> {
        let adapter: Arc<dyn RuntimeAdapter> = match cfg.runtime {
            RuntimeKind::Pty => Arc::new(MixedAdapter::new(&cfg)),
            RuntimeKind::Double => Arc::new(DoubleAdapter),
        };
        Self::with_adapter(cfg, db, adapter)
    }

    /// The daemon's state around a given runtime, for tests that need one
    /// the config cannot name.
    pub fn with_adapter(
        cfg: Config,
        db: Db,
        adapter: Arc<dyn RuntimeAdapter>,
    ) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&cfg.home)?;
        std::fs::create_dir_all(cfg.logs_dir())?;
        std::fs::create_dir_all(cfg.projects_dir())?;
        let secrets = Arc::new(Secrets::open(&cfg.secrets_dir())?);
        let events = Events::new();
        let auto_compact = AutoCompactOverride::default();
        if let Some(stored) = db.get_meta(AUTO_COMPACT_META_KEY)? {
            auto_compact.restore(&stored);
        }
        let supervisor = Supervisor::new(
            adapter,
            cfg.clone(),
            db.clone(),
            events.clone(),
            secrets.clone(),
            auto_compact.clone(),
        );
        let cfg_home = cfg.home.clone();
        let app = Arc::new(Self {
            cfg,
            db,
            events,
            secrets,
            supervisor,
            auto_compact,
            peers: crate::peer::PeerHub::default(),
            chat: crate::chat::ChatStore::default(),
            approvals: crate::approval::Approvals::default(),
            browsers: crate::browser::streams::BrowserStreams::default(),
            workers: crate::workers::Workers::default(),
            board: crate::board::feed::BoardFeed::default(),
            board_mirror: crate::board::mirror::BoardMirror::default(),
            bearers: crate::bus_auth::BearerLog::default(),
            owner: crate::bus_auth::owner::Owner::new(&cfg_home),
            started_at: Instant::now(),
            started_wall: SystemTime::now(),
        });
        // Every session start picks up what the last one left unfinished.
        let weak = Arc::downgrade(&app);
        app.supervisor.on_start(Box::new(move |bot_id, continues| {
            if let Some(app) = weak.upgrade() {
                crate::resume::pick_up(&app, bot_id, continues);
            }
        }));
        Ok(app)
    }

    /// True when the daemon binary on disk is newer than the running process.
    ///
    /// This is the rebuild-without-restart trap: the old process keeps serving
    /// the old MCP tool list, so a bot is told it cannot do something the
    /// source says it can, and everyone debugs the wrong layer. Surfacing it
    /// in diagnostics turns a silent capability gap into a visible "restart me".
    pub fn stale_build(&self) -> bool {
        std::env::current_exe()
            .and_then(|exe| exe.metadata()?.modified())
            .is_ok_and(|modified| modified > self.started_wall)
    }
}
