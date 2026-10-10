use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

mod overridden;
mod task_limits;

pub use overridden::{home_notice, home_override_var};
pub use task_limits::{TaskLimits, TaskLimitsConfig};

#[cfg(test)]
pub(crate) use overridden::overridden as env_home_overrides;

/// Bounds Claude Code accepts for the auto-compact window.
pub(crate) const AUTO_COMPACT_WINDOW_MIN: u32 = 100_000;
pub(crate) const AUTO_COMPACT_WINDOW_MAX: u32 = 1_000_000;
/// Comfortably above the 200k window a non-1M model assumes, so a bot chat
/// keeps far more history before it compacts, while still capping what any
/// single turn re-sends well below the 1M model limit.
pub(crate) const DEFAULT_AUTO_COMPACT_WINDOW: u32 = 250_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Root of all daemon state (`~/.thehermes`).
    pub home: PathBuf,
    /// Addresses to bind. Localhost plus, optionally, a Tailscale interface.
    pub bind: Vec<IpAddr>,
    /// The port actually being served, and the one bots are pointed at.
    /// Defaults to 49777 because Atlassian's managed Claude Code policy
    /// allowlists only that loopback `/mcp` URL; a bot session on a
    /// policy-managed Mac silently drops any MCP server whose exact URL is not
    /// allowlisted. Set from `configured_port` unless a fallback was
    /// negotiated at startup.
    pub port: u16,
    /// The port `hermesd.toml` asked for, kept so a negotiated fallback can be
    /// reported as the compromise it is rather than passed off as the setting.
    #[serde(skip)]
    pub configured_port: u16,
    /// Whether the app-managed service may fall back to another port when the
    /// configured one is occupied, restarting onto the configured one once it
    /// comes free. Turning this off trades a temporarily moved port for a
    /// daemon that refuses to start — the right trade on a policy-managed Mac,
    /// where only the allowlisted `/mcp` URL reaches bots at all. Ignored for
    /// direct launches, which always fail on a collision.
    pub negotiate_port: bool,
    /// Scratch mode for repros and rehearsals (H-171): no bot starts, no peer
    /// link, no routines, deliveries or workers, no bot-folder writes, no git
    /// push. Set by `--scratch` or `HERMES_SCRATCH=1`.
    pub scratch: bool,
    /// Take the owner acts a linked computer forwards (H-285, H-301). Never
    /// read from a config file: off in every daemon until signed approvals
    /// exist (owner ruling 6df14f7a); only tests of that path set it, through
    /// [`Config::trust_forwarded_owner_acts_for_tests`].
    #[serde(skip)]
    pub(crate) trust_forwarded_owner_acts: bool,
    /// The merge queue moves only when a test steps it, on the test's own
    /// clock: the daemon's 2 s ticker, on the real clock, would race it
    /// (H-308). Never read from a config file; see
    /// [`Config::merge_queue_by_hand_for_tests`].
    #[serde(skip)]
    pub(crate) merge_queue_by_hand: bool,
    /// `pty` (each bot's saved CLI) or `double` (deterministic test runtime).
    pub runtime: RuntimeKind,
    pub claude_bin: String,
    /// Extra arguments passed to the `claude` CLI.
    pub claude_args: Vec<String>,
    pub codex_bin: String,
    pub codex_args: Vec<String>,
    /// Folders outside their own that bots work in, such as the owner's
    /// repositories (`~` expands). Permission profiles name them to the
    /// auto-mode classifier as trusted; the guard hook lets destructive
    /// commands act only in their `<repo>-wt-*` git worktrees, never in the
    /// owner's checkouts beside them (H-031, CE-003 M4).
    pub trusted_paths: Vec<String>,
    /// Lines added to every bot's auto-mode `environment` (H-031), after the
    /// generated ones: the owner's own trust context, such as a tailnet
    /// domain or the repositories bots push to. This is where the lines of
    /// a hand-applied `bot-settings.json` belong once it is retired.
    pub auto_mode_environment: Vec<String>,
    /// Like `auto_mode_environment`, for the bots of one project only,
    /// keyed by the project's name.
    pub project_auto_mode_environment: std::collections::BTreeMap<String, Vec<String>>,
    pub default_bot_runtime: bus::BotRuntime,
    /// Context window, in tokens, a bot session may grow to before Claude Code
    /// auto-compacts it (`CLAUDE_CODE_AUTO_COMPACT_WINDOW`). Every turn of an
    /// always-on bot re-sends the whole transcript, so the model default (up
    /// to 1M) makes a long conversation expensive. `None` keeps that default.
    /// Claude Code accepts 100k–1M, so values are clamped, not dropped.
    pub auto_compact_window: Option<u32>,
    /// Pin Claude Code to its classic renderer, so xterm keeps the scrollback
    /// and the wheel scrolls it (`CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN`,
    /// `CLAUDE_CODE_DISABLE_MOUSE`).
    pub classic_renderer: bool,
    /// Per-project cap on how many bots may exist at once. The only limit on
    /// bot-authored bot creation: since bots cannot delete anything but their
    /// own children, and archived bots free their slot, this bounds the whole
    /// population regardless of how deeply bots nest their teams.
    pub max_bots_per_project: usize,
    /// Per-project cap on temporary workers running on this machine at once.
    /// Separate from `max_bots_per_project`, so a project whose permanent
    /// bots fill their cap can still fan work out; spawns past it wait in a
    /// queue until a worker finishes.
    pub max_workers_per_project: usize,
    /// How many tasks a bot may hold open per card and in all (H-125). Per
    /// computer: not synced to linked machines (ARCH-R59 c).
    pub tasks: TaskLimitsConfig,
    /// Projects, by name, whose code goes through pull requests (H-286,
    /// cut-over H-279): their bots' prompts carry the PR section from their
    /// next start. Empty until cut-over; taking a name out rolls it back.
    pub pr_flow_projects: Vec<String>,
    /// How long, in seconds, a permission prompt waits for an answer from the
    /// app before it is denied. Capped below the hook's own timeout.
    pub permission_timeout_seconds: u64,
    /// After a restart, tell each bot that was cut off mid-turn, or still
    /// holds open tasks, to pick its work back up. See [`crate::resume`].
    pub resume_after_restart: bool,
    pub delivery: DeliveryConfig,
    pub scheduler: SchedulerConfig,
    /// How often the supervisor reconciles live bots into running sessions.
    /// Bots are always-on, so this is the only thing that starts them.
    pub supervision_interval_ms: u64,
    /// Start staggering and the connect watchdog; see [`StartupConfig`].
    pub startup: crate::supervisor::StartupConfig,
    /// Terminal scrollback ring size per bot, in bytes.
    pub scrollback_bytes: usize,
    /// Extra allowed WebSocket Origin values. Requests without an Origin
    /// header (native clients) and tauri/localhost origins are always allowed.
    pub allowed_origins: Vec<String>,
    pub retention: RetentionConfig,
    /// Each bot's own browser. See [`crate::browser`].
    pub browser: crate::browser::BrowserConfig,
    /// Serving release builds on the tailnet (H-020 §6.6).
    pub releases: crate::board::release::serve::ServeConfig,
    /// How bots prove who they are on the bus (H-044).
    pub auth: crate::bus_auth::AuthConfig,
    /// Pausing every project for an install, and the services it stops (H-117).
    pub quiesce: crate::quiesce::services::QuiesceConfig,
    /// Where checks run: per-machine cap, disk floor, probe (H-283).
    pub checks: crate::prs::check_jobs::ChecksConfig,
    /// The *user's* home (Claude Code's `~/.claude/projects` transcripts), not
    /// the daemon's `home`; separate so tests can point it at a fixture tree.
    pub user_home: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionConfig {
    pub enabled: bool,
    pub message_days: i64,
    pub delivery_days: i64,
    pub routine_run_days: i64,
    /// How long emitted signals are kept once no run references them.
    pub signal_days: i64,
    /// How long identity revisions are kept. This is the undo buffer for
    /// changes bots make to themselves, so it outlives ordinary chat.
    pub revision_days: i64,
    /// How long an archived bot's workspace stays on disk before reclamation.
    pub archived_bot_days: i64,
    /// How often the pruning task runs.
    pub interval_hours: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            message_days: 180,
            delivery_days: 30,
            routine_run_days: 90,
            signal_days: 30,
            revision_days: 365,
            archived_bot_days: 30,
            interval_hours: 24,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Pty,
    Double,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DeliveryConfig {
    pub poll_interval_ms: u64,
    pub lease_seconds: i64,
    pub max_attempts: i64,
    pub base_backoff_seconds: i64,
    /// Type the owner's app chat into a bot's composer as their own turn
    /// (H-195 D2 + D2b, one switch). See `bus_auth::composer_delivery`.
    pub composer: bool,
    /// The same, for these bots only (by name or id) while `composer` is
    /// off: the live check on one test bot (H-209). Every other bot keeps
    /// inbox delivery.
    pub composer_bots: Vec<String>,
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1_000,
            lease_seconds: 60,
            max_attempts: 10,
            base_backoff_seconds: 5,
            composer: false,
            composer_bots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SchedulerConfig {
    pub tick_interval_ms: u64,
    /// Default deadline for one occurrence, when its routine sets none.
    pub lease_seconds: i64,
    /// How far back to schedule missed occurrences on startup.
    pub catch_up_window_minutes: i64,
    /// First retry delay for a failed occurrence; doubled per attempt.
    pub base_backoff_seconds: i64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            tick_interval_ms: 30_000,
            lease_seconds: 300,
            catch_up_window_minutes: 60,
            base_backoff_seconds: 30,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            home: default_home(),
            bind: vec![IpAddr::from([127, 0, 0, 1])],
            port: 49777,
            configured_port: 49777,
            negotiate_port: true,
            scratch: false,
            trust_forwarded_owner_acts: false,
            merge_queue_by_hand: false,
            runtime: RuntimeKind::Pty,
            claude_bin: "claude".to_string(),
            claude_args: Vec::new(),
            codex_bin: "codex".to_string(),
            codex_args: Vec::new(),
            trusted_paths: vec!["~/Developer".to_string()],
            auto_mode_environment: Vec::new(),
            project_auto_mode_environment: std::collections::BTreeMap::new(),
            default_bot_runtime: bus::BotRuntime::ClaudeCode,
            auto_compact_window: Some(DEFAULT_AUTO_COMPACT_WINDOW),
            classic_renderer: true,
            max_bots_per_project: 12,
            max_workers_per_project: bus::DEFAULT_MAX_WORKERS_PER_PROJECT,
            tasks: TaskLimitsConfig::default(),
            pr_flow_projects: Vec::new(),
            permission_timeout_seconds: 600,
            resume_after_restart: true,
            delivery: DeliveryConfig::default(),
            browser: crate::browser::BrowserConfig::default(),
            releases: Default::default(),
            auth: Default::default(),
            quiesce: Default::default(),
            checks: Default::default(),
            scheduler: SchedulerConfig::default(),
            supervision_interval_ms: 5_000,
            startup: Default::default(),
            scrollback_bytes: 1_048_576,
            allowed_origins: Vec::new(),
            retention: RetentionConfig::default(),
            user_home: dirs_home(),
        }
    }
}

/// Whether `THEHERMES_HOME` (or `GRAVITY_HOME`) points the daemon at a home
/// of the user's choosing; only the default home is migrated automatically.
/// Set to either default home, as the service launchers do, it is not.
pub fn home_is_overridden() -> bool {
    overridden::overridden(
        crate::brand::env_var_os("HOME")
            .map(PathBuf::from)
            .as_deref(),
        &dirs_home(),
    )
}

/// The home the daemon uses: the variable's home when it overrides, else the
/// default, so a launcher's variable naming `~/.gravity` cannot pin the
/// daemon to the home it is migrating away from.
pub fn default_home() -> PathBuf {
    resolve_home(
        crate::brand::env_var_os("HOME").map(PathBuf::from),
        dirs_home(),
    )
}

pub(crate) fn resolve_home(gravity_home: Option<PathBuf>, user_home: PathBuf) -> PathBuf {
    match gravity_home {
        Some(home) if overridden::overridden(Some(&home), &user_home) => home,
        _ => user_home.join(crate::brand::HOME_DIR_NAME),
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

impl Config {
    /// Whether `project` has cut over to pull requests (H-286).
    pub fn pr_flow(&self, project: &str) -> bool {
        self.pr_flow_projects
            .iter()
            .any(|p| p.eq_ignore_ascii_case(project))
    }

    /// Takes the owner acts a linked computer forwards: for tests of that
    /// path only (H-301, H-303). A release build has no way to set it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn trust_forwarded_owner_acts_for_tests(&mut self) {
        self.trust_forwarded_owner_acts = true;
    }

    /// Stops the daemon's own merge-queue ticker, so a test's
    /// `prs::queue::step(now)` is the only clock the queue sees (H-308).
    #[cfg(any(test, feature = "test-support"))]
    pub fn merge_queue_by_hand_for_tests(&mut self) {
        self.merge_queue_by_hand = true;
    }

    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let path = match path {
            Some(p) => p.to_path_buf(),
            None => default_home().join(crate::brand::daemon_file(".toml")),
        };
        let mut cfg = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let mut cfg: Config =
                toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
            cfg.configured_port = cfg.port;
            cfg
        } else {
            Config::default()
        };
        crate::board::release::confine::check_at_load(&mut cfg);
        // H-291 S3: 0 would stop every check as soon as it starts.
        anyhow::ensure!(
            cfg.checks.timeout_secs > 0,
            "checks.timeout_secs must be more than 0 in {}",
            path.display()
        );
        Ok(cfg)
    }

    pub fn db_path(&self) -> PathBuf {
        self.home.join("bus.sqlite")
    }

    pub fn secrets_dir(&self) -> PathBuf {
        self.home.join("secrets")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.home.join("logs")
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.home.join("projects")
    }

    /// The configured auto-compact window, clamped into the range Claude Code
    /// honours. Out-of-range values are ignored by the CLI, which would
    /// silently restore the expensive model default.
    pub fn effective_auto_compact_window(&self) -> Option<u32> {
        self.auto_compact_window
            .map(|w| w.clamp(AUTO_COMPACT_WINDOW_MIN, AUTO_COMPACT_WINDOW_MAX))
    }
}

#[cfg(test)]
mod tests;
