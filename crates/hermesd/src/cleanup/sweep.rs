//! The stale-worktree sweep (H-261 §15.5, CL-2): once a day each computer
//! looks at every linked worktree of its projects' repositories in the
//! allowed roots. A tree whose branch has no open PR, and that is merged
//! into main or idle for 14 days, goes under the same rules as a merged
//! PR's ([`super::run::attempt`]): clean and pushed, it is removed; dirty or
//! unpushed, it is salvaged and held, and its bot is told once; in no bot's
//! folder (an orphan), it is only reported. Main clones, the board home's
//! `git_cache`, release worktrees (`<repo>-rel-*`) and anything outside the
//! allowed roots are never touched: discovery only lists linked worktrees
//! of main clones in those roots, and a release tree is skipped by name.
//!
//! The board's home knows which branches have a live PR and which trees are
//! already held; a linked computer asks it (`cleanup_sweep_plan`) and sends
//! what it did back (`cleanup_swept`), and the home keeps every row.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::{json, Value};

use super::batch::bots_here;
use super::discover;
use super::model::Outcome;
use super::run::{self, Merge, Target};
use crate::app::AppState;
use crate::prs::model::PrState;
use crate::safe_git::SafeGit;

/// How often a computer looks whether its daily sweep is due.
pub const EVERY: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// A computer sweeps once this long after its last sweep.
pub const DAILY: Duration = Duration::hours(24);
/// A tree untouched this long is stale, merged or not.
pub const IDLE: Duration = Duration::days(14);
/// A merged tree counts once it is untouched this long, so a tree made
/// from main a minute ago (merged, trivially) isn't taken from its bot.
pub const SETTLED: Duration = Duration::days(1);

/// What the board's home tells a computer for one project's sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// The project works through PRs: only then does "no open PR" mean
    /// anything, so a project with none is never swept.
    pub uses_prs: bool,
    /// Branches with a live PR: never swept.
    pub live: Vec<String>,
    /// Trees on that computer already held or failed: left for the owner.
    pub skip: Vec<String>,
    /// Bots (by name) with a live PR: their build cache is kept.
    pub busy: Vec<String>,
}

impl Plan {
    pub fn to_json(&self) -> Value {
        json!({"uses_prs": self.uses_prs, "live": self.live, "skip": self.skip,
               "busy": self.busy})
    }

    pub fn from_json(v: &Value) -> Self {
        let list = |k: &str| -> Vec<String> {
            v[k].as_array()
                .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        Self {
            uses_prs: v["uses_prs"].as_bool() == Some(true),
            live: list("live"),
            skip: list("skip"),
            busy: list("busy"),
        }
    }
}

/// The plan for `project_id` on `machine`, from the board here.
pub fn plan_at_home(app: &AppState, project_id: &str, machine: &str) -> anyhow::Result<Plan> {
    let (prs, skip) = app
        .db
        .board_read(|t| Ok((t.prs(project_id, &PrState::ALL)?, t.sweep_skips(machine)?)))?;
    let live: Vec<_> = prs.iter().filter(|p| p.state.is_live()).collect();
    let mut busy: Vec<String> = live
        .iter()
        .filter_map(|p| app.db.get_bot(&p.author).ok().flatten())
        .map(|b| b.name)
        .collect();
    busy.sort();
    busy.dedup();
    Ok(Plan {
        uses_prs: !prs.is_empty(),
        live: live.iter().map(|p| p.branch.clone()).collect(),
        skip,
        busy,
    })
}

/// One tree the sweep looked at, and what became of it.
#[derive(Debug, Clone)]
pub struct Swept {
    pub path: String,
    pub main_clone: String,
    pub bot_id: Option<String>,
    pub bot_name: Option<String>,
    /// "merged into main" or "idle for 14 days".
    pub why: String,
    pub outcome: Outcome,
}

impl Swept {
    pub fn to_json(&self) -> Value {
        json!({"path": self.path, "main_clone": self.main_clone, "bot_id": self.bot_id,
               "bot_name": self.bot_name, "why": self.why, "outcome": self.outcome.to_json()})
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            path: v["path"].as_str()?.to_string(),
            main_clone: v["main_clone"].as_str()?.to_string(),
            bot_id: v["bot_id"].as_str().map(str::to_string),
            bot_name: v["bot_name"].as_str().map(str::to_string),
            why: v["why"].as_str().unwrap_or_default().to_string(),
            outcome: Outcome::from_json(&v["outcome"])?,
        })
    }
}

/// A release worktree (`<repo>-rel-<…>`): never swept.
fn is_release_tree(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains("-rel-"))
}

fn git(tree: &Path, args: &[&str]) -> Option<String> {
    SafeGit::local(tree).ok()?.args(args).run().ok()
}

/// When `tree` was last touched: its head commit, its index or its HEAD.
fn last_touched(tree: &Path) -> Option<DateTime<Utc>> {
    let commit = git(tree, &["log", "-1", "--format=%ct"])
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|s| Utc.timestamp_opt(s, 0).single());
    let dir = git(tree, &["rev-parse", "--absolute-git-dir"]).map(PathBuf::from);
    let mtime = |name: &str| -> Option<DateTime<Utc>> {
        let meta = std::fs::symlink_metadata(dir.as_ref()?.join(name)).ok()?;
        Some(meta.modified().ok()?.into())
    };
    [commit, mtime("index"), mtime("HEAD")]
        .into_iter()
        .flatten()
        .max()
}

/// Why `tree` is stale at `now`, if it is.
pub fn stale(tree: &Path, now: DateTime<Utc>) -> Option<String> {
    let touched = last_touched(tree)?;
    if now - touched >= IDLE {
        return Some("idle for 14 days".to_string());
    }
    let merged = git(
        tree,
        &[
            "merge-base",
            "--is-ancestor",
            "HEAD",
            "refs/remotes/origin/main",
        ],
    )
    .is_some();
    (merged && now - touched >= SETTLED).then(|| "merged into main".to_string())
}

/// The project's repositories here.
fn urls(app: &AppState, project_id: &str) -> Vec<String> {
    let own = app.db.project_repo(project_id).ok().flatten().map(|r| r.url);
    own.into_iter()
        .chain(app.db.extra_repos(project_id).unwrap_or_default())
        .collect()
}

/// One project's sweep on this computer.
pub fn sweep(app: &AppState, project_id: &str, plan: &Plan, now: DateTime<Utc>) -> Vec<Swept> {
    if !plan.uses_prs {
        return Vec::new();
    }
    let bots = bots_here(app, project_id);
    let skip: Vec<PathBuf> = plan
        .skip
        .iter()
        .map(|p| crate::safe_git::canonical(Path::new(p)).unwrap_or_else(|_| PathBuf::from(p)))
        .collect();
    let merge = Merge {
        merged_sha: "",
        salvage: app.cfg.home.join("salvage").join(project_id).join("sweep"),
    };
    let mut out = Vec::new();
    for url in urls(app, project_id) {
        for (found, branch) in discover::worktrees(app, &url, &bots) {
            let real = crate::safe_git::canonical(&found.path).unwrap_or(found.path.clone());
            if plan.live.contains(&branch) || is_release_tree(&found.path) || skip.contains(&real)
            {
                continue;
            }
            let Some(why) = stale(&found.path, now) else {
                continue;
            };
            let outcome = match &found.bot {
                None => Outcome::Held(format!(
                    "{} ({why}) is in no bot's folder: reported only",
                    found.path.display()
                )),
                Some(bot) => run::attempt(
                    app,
                    &Target {
                        path: found.path.clone(),
                        main_clone: Some(found.main_clone.clone()),
                        bot_name: bot.name.clone(),
                        workspace: bot.workspace.clone(),
                        no_other_work: !plan.busy.contains(&bot.name),
                    },
                    &merge,
                ),
            };
            // A tree in use isn't stale: it is looked at again tomorrow.
            if matches!(outcome, Outcome::Busy(_)) {
                continue;
            }
            out.push(Swept {
                path: real.display().to_string(),
                main_clone: found.main_clone.display().to_string(),
                bot_id: found.bot.as_ref().map(|b| b.id.clone()),
                bot_name: found.bot.map(|b| b.name),
                why,
                outcome,
            });
        }
    }
    out
}

/// Keeps what a computer's sweep did, on the board's home: a row per tree,
/// and a note to its bot (once: a held tree is skipped from then on).
pub fn record(app: &AppState, project_id: &str, machine: &str, swept: &[Swept], now: DateTime<Utc>) {
    let bots = app.db.list_bots(Some(project_id)).unwrap_or_default();
    for s in swept {
        let bot = s
            .bot_id
            .as_deref()
            .and_then(|id| bots.iter().find(|b| b.id == id))
            .or_else(|| {
                let name = s.bot_name.as_deref()?;
                bots.iter().find(|b| b.name == name)
            });
        let bot_id = bot.map(|b| b.id.as_str());
        let added = app.db.board_tx(|t| {
            let id = t.add_sweep_job(project_id, machine, &s.path, &s.main_clone, bot_id)?;
            t.cleanup_job(&id)
        });
        let job = match added {
            Ok(Some(job)) => job,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(path = %s.path, %error, "a swept tree wasn't recorded");
                continue;
            }
        };
        super::record(app, &job, &s.outcome, now);
        if let Some(bot) = bot_id {
            tell(app, bot, machine, s);
        }
    }
}

fn tell(app: &AppState, bot: &str, machine: &str, s: &Swept) {
    let body = match &s.outcome {
        Outcome::Done { bytes } => format!(
            "The daily cleanup removed your worktree {} on {machine} ({}; freed {}).",
            s.path,
            s.why,
            super::model::human_bytes(*bytes)
        ),
        Outcome::Held(why) | Outcome::Failed(why) => format!(
            "The daily cleanup kept your worktree {} on {machine} ({}): {why}. Nothing was \
             deleted.",
            s.path, s.why
        ),
        Outcome::Busy(_) => return,
    };
    if app.db.get_live_bot(bot).ok().flatten().is_none() {
        return;
    }
    let sender = crate::messaging::daemon_sender();
    let dm = crate::messaging::Dm::new(bot, &sender, bus::MessageKind::Note, &body);
    if let Err(error) = crate::messaging::send_dm(&app.db, &app.events, dm) {
        tracing::warn!(bot_id = %bot, %error, "couldn't tell a bot about its swept worktree");
    }
}

/// Salvage older than 30 days goes (ruling 7629a873): each saved tree's
/// folder under `<home>/salvage/<project>/<pr or sweep>/`.
pub fn prune_salvage(home: &Path, now: DateTime<Utc>) -> usize {
    const KEEP: Duration = Duration::days(30);
    let plain_dirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()))
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut gone = 0;
    for project in plain_dirs(&home.join("salvage")) {
        for group in plain_dirs(&project) {
            for saved in plain_dirs(&group) {
                let old = std::fs::symlink_metadata(&saved)
                    .and_then(|m| m.modified())
                    .is_ok_and(|at| now - DateTime::<Utc>::from(at) >= KEEP);
                if old && std::fs::remove_dir_all(&saved).is_ok() {
                    gone += 1;
                }
            }
        }
    }
    gone
}

pub fn spawn(app: Arc<AppState>) {
    // A scratch daemon removes nothing (H-171).
    if app.cfg.scratch {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(EVERY);
        loop {
            tick.tick().await;
            if let Err(error) = super::sweep_remote::if_due(&app, Utc::now()).await {
                tracing::warn!(%error, "the daily worktree sweep failed");
            }
        }
    });
}
