//! The jobs of one merged PR that one computer runs, the same on the board's
//! home and on a linked computer: each reported worktree, then whatever the
//! discovery finds on the branch that nobody reported.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::discover::{self, BotHere};
use super::model::{Kind, Outcome};
use super::run::{self, Merge, Target};
use crate::app::AppState;

/// One job as the request names it.
#[derive(Debug, Clone)]
pub struct Work {
    pub id: String,
    pub kind: Kind,
    pub path_or_ref: String,
    pub main_clone: Option<String>,
    pub bot_id: Option<String>,
    pub bot_name: String,
    pub no_other_work: bool,
}

/// One PR's jobs for this computer.
#[derive(Debug, Clone)]
pub struct Batch {
    /// The project here (a linked computer maps the home's id to its own).
    pub project_id: String,
    pub pr_id: String,
    pub pr_number: u32,
    pub branch: String,
    pub url: String,
    pub merged_sha: String,
    pub jobs: Vec<Work>,
    /// Paths already queued for this PR here, which discovery leaves alone.
    pub known: Vec<String>,
}

/// A worktree nobody reported, with what its attempt found.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub path: String,
    pub main_clone: String,
    pub bot_id: Option<String>,
    pub bot_name: Option<String>,
    pub outcome: Outcome,
}

#[derive(Debug, Default)]
pub struct Answer {
    pub results: Vec<(String, Outcome)>,
    pub discovered: Vec<Discovered>,
}

/// The bot's workspace on this computer: by its id, else (a linked
/// computer knows the home's bot by name) by its name in the project.
fn workspace_here(app: &AppState, project_id: &str, work: &Work) -> Option<PathBuf> {
    let by_id = work
        .bot_id
        .as_deref()
        .and_then(|id| app.db.get_bot(id).ok().flatten())
        .filter(|b| b.project_id == project_id);
    let bot = by_id.or_else(|| {
        app.db
            .list_bots(Some(project_id))
            .ok()?
            .into_iter()
            .find(|b| !work.bot_name.is_empty() && b.name == work.bot_name)
    })?;
    let ws = PathBuf::from(bot.workspace_path);
    ws.is_absolute().then_some(ws)
}

/// The project's bots as this computer knows them.
fn bots_here(app: &AppState, project_id: &str) -> Vec<BotHere> {
    app.db
        .list_bots(Some(project_id))
        .unwrap_or_default()
        .into_iter()
        .map(|b| {
            let ws = PathBuf::from(&b.workspace_path);
            BotHere {
                id: b.id,
                name: b.name,
                workspace: ws.is_absolute().then_some(ws),
            }
        })
        .collect()
}

/// Runs `batch` here: every rule for each worktree, in order.
pub fn run(app: &AppState, batch: &Batch) -> Answer {
    let merge = Merge {
        merged_sha: &batch.merged_sha,
        salvage: app
            .cfg
            .home
            .join("salvage")
            .join(&batch.project_id)
            .join(batch.pr_number.to_string()),
    };
    let mut answer = Answer::default();
    for work in &batch.jobs {
        let outcome = match work.kind {
            Kind::Worktree => run::attempt(
                app,
                &Target {
                    path: PathBuf::from(&work.path_or_ref),
                    main_clone: work.main_clone.as_ref().map(PathBuf::from),
                    bot_name: work.bot_name.clone(),
                    workspace: workspace_here(app, &batch.project_id, work),
                    no_other_work: work.no_other_work,
                },
                &merge,
            ),
            Kind::Discover => {
                answer.discovered.extend(discover_here(app, batch, &merge));
                Outcome::Done { bytes: 0 }
            }
        };
        answer.results.push((work.id.clone(), outcome));
    }
    answer
}

/// The branch's worktrees here that no job names yet, each attempted.
fn discover_here(app: &AppState, batch: &Batch, merge: &Merge<'_>) -> Vec<Discovered> {
    let bots = bots_here(app, &batch.project_id);
    let known: Vec<PathBuf> = batch
        .known
        .iter()
        .chain(batch.jobs.iter().map(|w| &w.path_or_ref))
        .filter_map(|p| crate::safe_git::canonical(std::path::Path::new(p)).ok())
        .collect();
    let mut out = Vec::new();
    for found in discover::on_branch(app, &batch.url, &batch.branch, &bots) {
        let real = crate::safe_git::canonical(&found.path).unwrap_or(found.path.clone());
        if known.contains(&real) {
            continue;
        }
        let outcome = match &found.bot {
            None => Outcome::Held(format!(
                "{} is on {} but in no bot's folder: reported only",
                found.path.display(),
                batch.branch
            )),
            Some(bot) => {
                // The cache is trimmed only for a bot the home vouched for.
                let no_other_work = batch
                    .jobs
                    .iter()
                    .any(|w| w.bot_id.as_deref() == Some(&bot.id) && w.no_other_work);
                run::attempt(
                    app,
                    &Target {
                        path: found.path.clone(),
                        main_clone: Some(found.main_clone.clone()),
                        bot_name: bot.name.clone(),
                        workspace: bot.workspace.clone(),
                        no_other_work,
                    },
                    merge,
                )
            }
        };
        out.push(Discovered {
            path: real.display().to_string(),
            main_clone: found.main_clone.display().to_string(),
            bot_id: found.bot.as_ref().map(|b| b.id.clone()),
            bot_name: found.bot.map(|b| b.name),
            outcome,
        });
    }
    out
}

impl Batch {
    /// The `cleanup_request` frame for a linked computer.
    pub fn to_frame(&self, home_project: &str) -> Value {
        let jobs: Vec<Value> = self
            .jobs
            .iter()
            .map(|w| {
                json!({"id": w.id, "kind": w.kind.as_str(), "path": w.path_or_ref,
                       "main_clone": w.main_clone, "bot_id": w.bot_id, "bot_name": w.bot_name,
                       "no_other_work": w.no_other_work})
            })
            .collect();
        json!({
            "type": super::remote::REQUEST, "project_id": home_project, "pr": self.pr_id,
            "number": self.pr_number, "branch": self.branch, "url": self.url,
            "merged_sha": self.merged_sha, "jobs": jobs, "known": self.known,
        })
    }

    /// A request read back, for the project `project_id` here.
    pub fn from_frame(frame: &Value, project_id: &str, url: String) -> Self {
        let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
        let jobs = frame["jobs"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or_default()
            .iter()
            .filter_map(|j| {
                Some(Work {
                    id: text(&j["id"]),
                    kind: Kind::parse(j["kind"].as_str()?)?,
                    path_or_ref: text(&j["path"]),
                    main_clone: j["main_clone"].as_str().map(str::to_string),
                    bot_id: j["bot_id"].as_str().map(str::to_string),
                    bot_name: text(&j["bot_name"]),
                    no_other_work: j["no_other_work"].as_bool() == Some(true),
                })
            })
            .collect();
        Self {
            project_id: project_id.to_string(),
            pr_id: text(&frame["pr"]),
            pr_number: frame["number"].as_u64().unwrap_or_default() as u32,
            branch: text(&frame["branch"]),
            url,
            merged_sha: text(&frame["merged_sha"]),
            jobs,
            known: frame["known"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

impl Answer {
    /// The `cleanup_result` frame back to the board's home.
    pub fn to_frame(&self, home_project: &str, pr_id: &str) -> Value {
        let results: Vec<Value> = self
            .results
            .iter()
            .map(|(id, o)| json!({"id": id, "outcome": o.to_json()}))
            .collect();
        let discovered: Vec<Value> = self
            .discovered
            .iter()
            .map(|d| {
                json!({"path": d.path, "main_clone": d.main_clone, "bot_id": d.bot_id,
                       "bot_name": d.bot_name, "outcome": d.outcome.to_json()})
            })
            .collect();
        json!({"type": super::remote::RESULT, "project_id": home_project, "pr": pr_id,
               "results": results, "discovered": discovered})
    }
}
