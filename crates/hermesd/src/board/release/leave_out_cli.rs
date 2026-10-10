//! `hermesd release leave-out <id>` (H-272; UX-051 decision 10): DevOps, on
//! the daemon's task, reverts the PRs the owner left out of a release.
//!
//! The daemon says what (`hermes/leave_out`): the branch name and, newest
//! first, each PR's merged range. In a fresh worktree of DevOps' checkout
//! at origin's main (git hooks off), each range is reverted with
//! `git revert --no-edit <from>..<to>`; the branch is pushed (never forced)
//! and the worktree removed. The daemon then opens it as the owner's PR
//! (`hermes/leave_out_pushed`), which merges through the normal queue.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::git::{self, git as run_git, no_hooks};
use super::land::ask;
use crate::config::Config;
use crate::prs::repo;

const USAGE: &str = "usage: hermesd release leave-out <leave-out id>";

/// What to revert, as the daemon said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub branch: String,
    pub repo_url: String,
    /// (number, from, to), newest first.
    pub reverts: Vec<(u64, String, String)>,
}

impl Plan {
    pub fn from_gate(gate: &Value) -> anyhow::Result<Self> {
        let text = |v: &Value, key: &str| {
            v[key]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("the daemon named no {key}"))
        };
        let reverts = gate["reverts"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("the daemon named no PRs"))?
            .iter()
            .map(|r| {
                Ok((
                    r["number"].as_u64().unwrap_or(0),
                    text(r, "from")?,
                    text(r, "to")?,
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            branch: text(gate, "branch")?,
            repo_url: text(gate, "repo_url")?,
            reverts,
        })
    }
}

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let [id] = args else {
        anyhow::bail!("{USAGE}");
    };
    let gate = ask(cfg, "hermes/leave_out", json!({ "id": id })).await?;
    let plan = Plan::from_gate(&gate)?;
    let checkout = git::toplevel(&std::env::current_dir()?)?;
    let head = revert_in(&checkout, &plan, &std::env::temp_dir())?;
    let reply = ask(
        cfg,
        "hermes/leave_out_pushed",
        json!({ "id": id, "branch": plan.branch, "head": head }),
    )
    .await?;
    println!(
        "reverted on {} at {head}; it is the owner's PR #{} now, and merges through the queue",
        plan.branch,
        reply["number"].as_u64().unwrap_or_default()
    );
    Ok(())
}

/// The git half: reverts in a fresh worktree under `scratch`, pushes the
/// branch, removes the worktree. Returns the pushed head.
pub fn revert_in(checkout: &Path, plan: &Plan, scratch: &Path) -> anyhow::Result<String> {
    let origin = run_git(checkout, &["remote", "get-url", "origin"])?;
    anyhow::ensure!(
        repo::same(&origin, &plan.repo_url),
        "this checkout's origin is {origin}, not {}; run it in a checkout of that repo",
        plan.repo_url
    );
    git::fetch(checkout, &["main"])?;
    let tree: PathBuf = scratch.join(format!("hermes-leave-out-{}", uuid::Uuid::new_v4()));
    let path = tree.display().to_string();
    run_git(
        checkout,
        &[
            "worktree",
            "add",
            "--detach",
            &path,
            "refs/remotes/origin/main",
        ],
    )?;
    let result = revert_each(&tree, plan).and_then(|()| {
        git::push(&tree, &[format!("HEAD:refs/heads/{}", plan.branch)])?;
        run_git(&tree, &["rev-parse", "HEAD"])
    });
    let _ = run_git(checkout, &["worktree", "remove", "--force", &path]);
    result
}

fn revert_each(tree: &Path, plan: &Plan) -> anyhow::Result<()> {
    for (number, from, to) in &plan.reverts {
        let range = format!("{from}..{to}");
        let mut args: Vec<&str> = no_hooks().to_vec();
        args.extend(["revert", "--no-edit", &range]);
        if let Err(e) = run_git(tree, &args) {
            let _ = run_git(tree, &["revert", "--abort"]);
            anyhow::bail!(
                "PR #{number} doesn't revert cleanly on main, so nothing was pushed: {e}"
            );
        }
    }
    Ok(())
}
