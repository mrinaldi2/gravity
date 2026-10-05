//! The startup guards against a fake Claude Code runtime that comes up but
//! never reports its inbox socket — what a lost SessionStart hook looks like.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::overrides::AutoCompactOverride;
use crate::runtime::double::DoubleAdapter;
use crate::runtime::{Capabilities, StartedSession};

/// The double, minus the socket it would report: only a SessionStart hook
/// could tell the daemon where to deliver, and none ever comes.
#[derive(Default)]
struct SilentAdapter {
    starts: AtomicUsize,
}

impl RuntimeAdapter for SilentAdapter {
    fn capabilities(&self) -> Capabilities {
        DoubleAdapter.capabilities()
    }

    fn start(&self, spec: &BotSpec) -> anyhow::Result<StartedSession> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let mut started = DoubleAdapter.start(spec)?;
        started.msg_socket = None;
        Ok(started)
    }

    fn probe(&self) -> anyhow::Result<String> {
        DoubleAdapter.probe()
    }
}

struct Fixture {
    sup: Supervisor,
    adapter: Arc<SilentAdapter>,
    db: Db,
    project_id: String,
    home: tempfile::TempDir,
}

impl Fixture {
    fn new(tweak: impl FnOnce(&mut Config)) -> Self {
        let home = tempfile::tempdir().expect("tempdir");
        let mut cfg = Config {
            home: home.path().to_path_buf(),
            user_home: home.path().join("user"),
            ..Config::default()
        };
        cfg.startup.connect_timeout_ms = 50;
        tweak(&mut cfg);
        let db = Db::open(&home.path().join("bus.sqlite")).expect("db");
        let project_id = db.create_project("p", "p").expect("project").id;
        let adapter = Arc::new(SilentAdapter::default());
        let secrets = Arc::new(Secrets::open(&home.path().join("secrets")).expect("secrets"));
        let sup = Supervisor::new(
            adapter.clone(),
            cfg,
            db.clone(),
            Events::new(),
            secrets,
            AutoCompactOverride::default(),
        );
        Self {
            sup,
            adapter,
            db,
            project_id,
            home,
        }
    }

    /// A bot whose workspace does not exist, so no trust step runs.
    fn bot(&self, name: &str, runtime: bus::BotRuntime) -> String {
        let workspace = self.home.path().join(format!("p/bots/{name}/workspace"));
        self.db
            .create_bot_with_runtime(
                &self.project_id,
                name,
                "",
                "",
                "",
                &workspace.display().to_string(),
                name,
                None,
                runtime,
            )
            .expect("bot")
            .id
    }

    fn starts(&self) -> usize {
        self.adapter.starts.load(Ordering::SeqCst)
    }

    /// Supervision ticks until `done` holds, failing after a few seconds.
    async fn tick_until(&self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            self.sup.reconcile();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

fn gave_up(sup: &Supervisor, bot_id: &str) -> bool {
    sup.state(bot_id) == (BotState::WaitingForUser, DIDNT_CONNECT.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_never_connects_is_restarted_then_given_up_on() {
    let f = Fixture::new(|_| {});
    let mut pushes = f.sup.inner.events.subscribe_push();
    let bot_id = f.bot("alice", bus::BotRuntime::ClaudeCode);

    // One start plus the two watchdog restarts, then the owner is asked.
    f.tick_until("the watchdog to give up", |f| gave_up(&f.sup, &bot_id))
        .await;
    assert_eq!(f.starts(), 3);

    // Given up means given up: further ticks restart nothing.
    for _ in 0..10 {
        f.sup.reconcile();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(f.starts(), 3);
    assert!(gave_up(&f.sup, &bot_id));

    let mut titles = Vec::new();
    while let Ok(push) = pushes.try_recv() {
        if let Push::Notify { title, .. } = push {
            titles.push(title);
        }
    }
    assert_eq!(titles, ["alice didn't connect"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owners_restart_gives_the_watchdog_its_budget_back() {
    let f = Fixture::new(|_| {});
    let bot_id = f.bot("alice", bus::BotRuntime::ClaudeCode);
    f.tick_until("the watchdog to give up", |f| gave_up(&f.sup, &bot_id))
        .await;

    f.sup.restart_bot(&bot_id).expect("restart");
    f.tick_until("a second give-up", |f| {
        f.starts() == 6 && gave_up(&f.sup, &bot_id)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connected_session_is_left_alone_and_recovers_from_a_give_up() {
    let f = Fixture::new(|_| {});
    let bot_id = f.bot("alice", bus::BotRuntime::ClaudeCode);
    f.tick_until("the watchdog to give up", |f| gave_up(&f.sup, &bot_id))
        .await;

    // The socket arriving late still makes the bot reachable and idle.
    f.sup.set_msg_socket(&bot_id, "/tmp/late.sock", None);
    assert_eq!(f.sup.state(&bot_id).0, BotState::Ready);
    for _ in 0..10 {
        f.sup.reconcile();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(f.starts(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_in_flight_is_not_cut_off() {
    let f = Fixture::new(|_| {});
    let bot_id = f.bot("alice", bus::BotRuntime::ClaudeCode);
    f.tick_until("the first start", |f| f.starts() == 1).await;
    f.sup.on_hook(&bot_id, "UserPromptSubmit", None);

    for _ in 0..10 {
        f.sup.reconcile();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(f.starts(), 1);
    assert_eq!(f.sup.state(&bot_id).0, BotState::Working);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mass_start_is_staggered_but_codex_is_not_held_back() {
    let f = Fixture::new(|cfg| {
        cfg.startup.connect_timeout_ms = 60_000;
        cfg.startup.max_concurrent_starts = 3;
    });
    let claude: Vec<String> = (0..8)
        .map(|i| f.bot(&format!("bot{i}"), bus::BotRuntime::ClaudeCode))
        .collect();
    let codex = f.bot("codex", bus::BotRuntime::CodexCli);

    f.sup.reconcile();
    assert!(f.sup.has_session(&codex));
    let up = |f: &Fixture| claude.iter().filter(|id| f.sup.has_session(id)).count();
    assert_eq!(up(&f), 3);
    f.sup.reconcile();
    assert_eq!(up(&f), 3, "no slot frees until a session connects");

    for id in claude.iter().filter(|id| f.sup.has_session(id)) {
        f.sup.set_msg_socket(id, "/tmp/up.sock", None);
    }
    f.sup.reconcile();
    assert_eq!(up(&f), 6);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_session_stops_holding_its_slot_after_the_warmup() {
    let f = Fixture::new(|cfg| {
        cfg.startup.connect_timeout_ms = 60_000;
        cfg.startup.max_concurrent_starts = 1;
        cfg.startup.warmup_ms = 50;
    });
    for i in 0..3 {
        f.bot(&format!("bot{i}"), bus::BotRuntime::ClaudeCode);
    }
    f.sup.reconcile();
    assert_eq!(f.starts(), 1);
    f.tick_until("every bot to start", |f| f.starts() == 3)
        .await;
}

/// A runtime whose start for `hung` never returns until released.
struct HangingAdapter {
    starts: AtomicUsize,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl RuntimeAdapter for HangingAdapter {
    fn capabilities(&self) -> Capabilities {
        DoubleAdapter.capabilities()
    }

    fn start(&self, spec: &BotSpec) -> anyhow::Result<StartedSession> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if spec.bot_name == "hung" {
            let _ = self.release.lock().expect("release").recv();
        }
        DoubleAdapter.start(spec)
    }

    fn probe(&self) -> anyhow::Result<String> {
        DoubleAdapter.probe()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hung_start_stops_holding_its_slot_after_the_warmup() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut cfg = Config {
        home: home.path().to_path_buf(),
        user_home: home.path().join("user"),
        ..Config::default()
    };
    cfg.startup.connect_timeout_ms = 60_000;
    cfg.startup.max_concurrent_starts = 1;
    // Long enough that no scheduler stall can run it out before the slot is
    // checked; the test ends the warmup itself by backdating the start.
    let warmup = Duration::from_secs(60);
    cfg.startup.warmup_ms = warmup.as_millis() as u64;
    let db = Db::open(&home.path().join("bus.sqlite")).expect("db");
    let project_id = db.create_project("p", "p").expect("project").id;
    let (release, rx) = std::sync::mpsc::channel();
    let adapter = Arc::new(HangingAdapter {
        starts: AtomicUsize::new(0),
        release: Mutex::new(rx),
    });
    let secrets = Arc::new(Secrets::open(&home.path().join("secrets")).expect("secrets"));
    let sup = Supervisor::new(
        adapter.clone(),
        cfg,
        db.clone(),
        Events::new(),
        secrets,
        AutoCompactOverride::default(),
    );
    let ids: Vec<String> = ["hung", "next"]
        .iter()
        .map(|name| {
            let workspace = home.path().join(format!("p/bots/{name}/workspace"));
            db.create_bot(
                &project_id,
                name,
                "",
                "",
                "",
                &workspace.display().to_string(),
                name,
                None,
            )
            .expect("bot")
            .id
        })
        .collect();
    let starts = || adapter.starts.load(Ordering::SeqCst);

    let hung = {
        let (sup, id) = (sup.clone(), ids[0].clone());
        tokio::task::spawn_blocking(move || sup.start_bot(&id))
    };
    while starts() == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    sup.reconcile();
    assert_eq!(starts(), 1, "the hung start's slot was not held");

    // The warmup runs out: the hung start began a full warmup ago.
    sup.lock_bots()
        .get_mut(&ids[0])
        .expect("hung handle")
        .starting_since = Instant::now().checked_sub(warmup).expect("backdate");
    sup.reconcile();
    assert!(sup.has_session(&ids[1]), "the next bot never started");
    release.send(()).expect("release");
    hung.await.expect("join").expect("hung start");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_config_lock_defers_the_start_and_retries() {
    let f = Fixture::new(|cfg| cfg.startup.connect_timeout_ms = 60_000);
    let bot_id = f.bot("alice", bus::BotRuntime::ClaudeCode);
    let bot = f.db.get_bot(&bot_id).expect("db").expect("bot");
    std::fs::create_dir_all(&bot.workspace_path).expect("workspace");
    let user_home = f.home.path().join("user");
    std::fs::create_dir_all(&user_home).expect("user home");
    // Another Claude Code process holds the lock, and keeps it fresh.
    let config = crate::paths::claude_config_path(&user_home);
    let lock = std::path::PathBuf::from(format!("{}.lock", config.display()));
    std::fs::create_dir(&lock).expect("lock");

    f.sup.reconcile();
    assert_eq!(f.starts(), 0, "a start into the trust dialog was launched");
    f.sup.reconcile();
    assert_eq!(f.starts(), 0, "the retry is backed off, not run every tick");

    std::fs::remove_dir(&lock).expect("unlock");
    f.tick_until("the deferred start", |f| f.starts() == 1)
        .await;
    let written = std::fs::read_to_string(&config).expect("config written");
    assert!(written.contains("hasTrustDialogAccepted"), "{written}");
}
