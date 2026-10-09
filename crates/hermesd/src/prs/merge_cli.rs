//! `hermesd pr merge <n> [--dry-run]` (H-284; H-261 §5.2): DevOps, on the
//! daemon's `pr_merge` task, fast-forwards `main` to exactly the PR's head.
//!
//! The daemon checks first (`hermes/pr_merge`, see `prs::merge`): the
//! `pr_merge` extra, the DevOps role, the task, and every §5.1 condition
//! with the needs read again at the merge. Then, in DevOps' own checkout,
//! with git hooks off:
//! - the checkout's origin must be the PR's repo;
//! - the branch on origin must still be at the head, and main must be an
//!   ancestor of it: a fast-forward, never forced;
//! - `<head>:refs/heads/main` is pushed, and the repo itself is asked that
//!   main is at the head;
//! - the branch is deleted only while its tip is still the merged commit
//!   (a lease); otherwise it is kept and reported;
//! - the daemon records it (`hermes/pr_merged`) after its own fetch.
//!
//! A raw `git push … main` still goes through the guard, unchanged.

use std::path::Path;

use serde_json::{json, Value};

use crate::board::release::git::{self, git as run_git};
use crate::board::release::land::ask;
use crate::config::Config;
use crate::prs::repo;

const USAGE: &str = "usage: hermesd pr merge <number> [--dry-run]";

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let (sub, rest) = args
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("{USAGE}"))?;
    anyhow::ensure!(sub == "merge", "{USAGE}");
    let (number, dry_run) = parse(rest)?;
    let gate = ask(cfg, "hermes/pr_merge", json!({ "number": number })).await?;
    let plan = Plan::from_gate(&gate, dry_run)?;
    let here = git::toplevel(&std::env::current_dir()?)?;
    let done = merge_in(&here, &plan)?;
    if dry_run {
        println!(
            "dry run: would fast-forward main on {} to {}",
            plan.repo, plan.head
        );
        return Ok(());
    }
    let reply = ask(
        cfg,
        "hermes/pr_merged",
        json!({ "number": number, "sha": plan.head,
                "branch": { "deleted": done.branch_deleted, "note": done.branch_note } }),
    )
    .await?;
    println!(
        "PR #{number}: main on {} is at {} (pushed over {})",
        plan.repo, plan.head, done.transport
    );
    println!("branch {}: {}", plan.branch, done.branch_note);
    if reply["moved"].as_bool() == Some(true) {
        println!(
            "{} moved to Verify",
            reply["card"].as_str().unwrap_or("the card")
        );
    } else if let Some(waiting) = reply["waiting_for"].as_array().filter(|w| !w.is_empty()) {
        let names: Vec<&str> = waiting.iter().filter_map(Value::as_str).collect();
        println!(
            "{} waits for {} before it moves to Verify",
            reply["card"].as_str().unwrap_or("the card"),
            names.join(", ")
        );
    }
    Ok(())
}

fn parse(args: &[String]) -> anyhow::Result<(u32, bool)> {
    let mut number = None;
    let mut dry_run = false;
    for arg in args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            a if number.is_none() => {
                number = Some(
                    a.trim_start_matches('#')
                        .parse::<u32>()
                        .map_err(|_| anyhow::anyhow!("{a:?} isn't a PR number; {USAGE}"))?,
                )
            }
            _ => anyhow::bail!("{USAGE}"),
        }
    }
    Ok((number.ok_or_else(|| anyhow::anyhow!("{USAGE}"))?, dry_run))
}

/// What the daemon said to merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub head: String,
    pub branch: String,
    /// "owner/name".
    pub repo: String,
    /// Where the result is checked.
    pub repo_url: String,
    pub dry_run: bool,
}

impl Plan {
    fn from_gate(gate: &Value, dry_run: bool) -> anyhow::Result<Self> {
        let text = |key: &str| {
            gate[key]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("the daemon named no {key}"))
        };
        Ok(Self {
            head: text("head_sha")?,
            branch: text("branch")?,
            repo: text("repo")?,
            repo_url: text("repo_url")?,
            dry_run,
        })
    }
}

/// What the git half did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub transport: &'static str,
    pub branch_deleted: bool,
    pub branch_note: String,
}

/// The git half, in `checkout`: checks, the fast-forward, the remote's
/// word on it, then the branch.
pub fn merge_in(checkout: &Path, plan: &Plan) -> anyhow::Result<Done> {
    let origin = run_git(checkout, &["remote", "get-url", "origin"])?;
    anyhow::ensure!(
        repo::same(&origin, &plan.repo_url),
        "this checkout's origin is {origin}, not {}; run it in a checkout of that repo",
        plan.repo
    );
    let transport = git::fetch(checkout, &["main", &plan.branch])?;
    let tip = run_git(
        checkout,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/remotes/origin/{}^{{commit}}", plan.branch),
        ],
    )
    .map_err(|_| anyhow::anyhow!("no branch {} on origin", plan.branch))?;
    anyhow::ensure!(
        tip == plan.head,
        "{} is at {tip}, not the PR's head {}: nothing pushed",
        plan.branch,
        plan.head
    );
    anyhow::ensure!(
        git::yes(
            checkout,
            &[
                "merge-base",
                "--is-ancestor",
                "refs/remotes/origin/main",
                &plan.head
            ]
        ),
        "main has commits that {} lacks: not a fast-forward, so nothing was pushed",
        plan.head
    );
    if plan.dry_run {
        return Ok(Done {
            transport,
            branch_deleted: false,
            branch_note: "dry run".to_string(),
        });
    }
    let transport = git::push(checkout, &[format!("{}:refs/heads/main", plan.head)])?;
    let main = remote_tip(&plan.repo_url, "main")?;
    anyhow::ensure!(
        main.as_deref() == Some(plan.head.as_str()),
        "pushed, but {} has main at {}, not {}; nothing recorded",
        plan.repo,
        main.as_deref().unwrap_or("nothing"),
        plan.head
    );
    let (branch_deleted, branch_note) = drop_branch(checkout, plan);
    Ok(Done {
        transport,
        branch_deleted,
        branch_note,
    })
}

fn remote_tip(url: &str, branch: &str) -> anyhow::Result<Option<String>> {
    let name = format!("refs/heads/{branch}");
    Ok(git::remote_refs(url, &[name.clone()])?
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, sha)| sha))
}

/// Deletes the remote branch only while it is still at the merged commit.
fn drop_branch(checkout: &Path, plan: &Plan) -> (bool, String) {
    match remote_tip(&plan.repo_url, &plan.branch) {
        Ok(Some(tip)) if tip != plan.head => {
            return (false, format!("kept: it moved to {tip} after the merge"))
        }
        Ok(None) => return (true, "already gone".to_string()),
        Err(e) => return (false, format!("kept: couldn't read it ({e})")),
        Ok(Some(_)) => {}
    }
    match git::delete_branch(checkout, &plan.branch, &plan.head) {
        Ok(_) => (true, "deleted".to_string()),
        Err(e) => (false, format!("kept: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn a_number_and_dry_run() {
        assert_eq!(parse(&["7".into()]).unwrap(), (7, false));
        assert_eq!(
            parse(&["#7".into(), "--dry-run".into()]).unwrap(),
            (7, true)
        );
        assert!(parse(&[]).is_err());
        assert!(parse(&["x".into()]).is_err());
        assert!(parse(&["7".into(), "8".into()]).is_err());
    }
}
