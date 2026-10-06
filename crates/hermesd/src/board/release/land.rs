//! `hermesd release land <release> [--commit <sha>] [--branch <b>]
//! [--tag <t>] [--dry-run]` (H-117 X2): DevOps puts an approved release on
//! `main` and tags it, with no owner step.
//!
//! The daemon checks first (`hermes/release_land`): the `release_main`
//! extra, the project's DevOps role, a release the owner approved. Then, in
//! DevOps's own checkout:
//! - the commit (the release branch's tip unless named) must be on the
//!   release branch, `release/desktop-<version>` unless named;
//! - `main` must fast-forward to it: never forced, a diverged main is
//!   refused;
//! - `main` is pushed, then the annotated tag (`desktop-v<version>` unless
//!   named), and both go on the release (`hermes/release_landed`).
//!
//! Over SSH, or GitHub over HTTPS with gh's credential when SSH has no key.
//! A raw `git push … main` still goes through the guard, unchanged.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use super::git::{self, git as run_git};
use crate::config::Config;

const USAGE: &str = "usage: hermesd release land <release> [--commit <sha>] [--branch <name>] \
                     [--tag <name>] [--dry-run]";

/// What to land, once the daemon said who and which release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub release: String,
    pub version: String,
    pub commit: Option<String>,
    pub branch: String,
    pub tag: String,
    pub dry_run: bool,
}

/// What landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landed {
    pub commit: String,
    pub tag: String,
    /// How it went out: `origin`, or GitHub over HTTPS.
    pub transport: &'static str,
}

/// Flags, before the daemon names the version the defaults depend on.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Args {
    pub release: String,
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub dry_run: bool,
}

pub(crate) fn parse(args: &[String]) -> anyhow::Result<Args> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--commit" => out.commit = it.next().cloned(),
            "--branch" => out.branch = it.next().cloned(),
            "--tag" => out.tag = it.next().cloned(),
            "--dry-run" => out.dry_run = true,
            a if a.starts_with("--") || !out.release.is_empty() => anyhow::bail!("{USAGE}"),
            a => out.release = a.to_string(),
        }
    }
    anyhow::ensure!(!out.release.is_empty(), "{USAGE}");
    Ok(out)
}

impl Args {
    /// The plan, with the repo's own conventions for what isn't named.
    pub(crate) fn plan(self, version: &str) -> Plan {
        Plan {
            branch: self
                .branch
                .unwrap_or_else(|| format!("release/desktop-{version}")),
            tag: self.tag.unwrap_or_else(|| format!("desktop-v{version}")),
            release: self.release,
            version: version.to_string(),
            commit: self.commit,
            dry_run: self.dry_run,
        }
    }
}

/// Asks the daemon over the local endpoint, as the bot of this session.
pub(crate) async fn ask(cfg: &Config, method: &str, params: Value) -> anyhow::Result<Value> {
    let endpoint = crate::bus_auth::ipc::endpoint(cfg).display().to_string();
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let reply =
        crate::bus_auth::hook::send(&endpoint, &request, false, Duration::from_secs(30)).await?;
    if let Some(error) = reply.get("error") {
        anyhow::bail!(
            "{} (run this from your own session)",
            error["message"].as_str().unwrap_or("refused")
        );
    }
    Ok(reply["result"].clone())
}

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let args = parse(args)?;
    let gate = ask(
        cfg,
        "hermes/release_land",
        json!({ "release_id": args.release }),
    )
    .await?;
    let version = gate["version"].as_str().unwrap_or_default().to_string();
    let plan = args.plan(&version);
    let repo = git::toplevel(&std::env::current_dir()?)?;
    let landed = land_in(&repo, &plan)?;
    if plan.dry_run {
        println!(
            "dry run: would fast-forward main to {} and tag it {}",
            landed.commit, landed.tag
        );
        return Ok(());
    }
    ask(
        cfg,
        "hermes/release_landed",
        json!({ "release_id": plan.release, "commit": landed.commit, "tag": landed.tag }),
    )
    .await?;
    println!(
        "release {}: main is at {} and tagged {} (pushed over {})",
        plan.release, landed.commit, landed.tag, landed.transport
    );
    Ok(())
}

/// The git half: checks, then main and the tag, in `repo`.
pub fn land_in(repo: &Path, plan: &Plan) -> anyhow::Result<Landed> {
    let transport = git::fetch(repo, &["main", &plan.branch])?;
    let tip = format!("refs/remotes/origin/{}", plan.branch);
    let named = plan.commit.as_deref().unwrap_or(&tip);
    let commit = run_git(
        repo,
        &["rev-parse", "--verify", &format!("{named}^{{commit}}")],
    )
    .map_err(|_| anyhow::anyhow!("no commit {named} (is {} pushed?)", plan.branch))?;
    anyhow::ensure!(
        git::yes(repo, &["merge-base", "--is-ancestor", &commit, &tip]),
        "{commit} isn't on {}: land only what the release branch holds",
        plan.branch
    );
    anyhow::ensure!(
        git::yes(
            repo,
            &[
                "merge-base",
                "--is-ancestor",
                "refs/remotes/origin/main",
                &commit
            ]
        ),
        "main has commits that {commit} lacks: not a fast-forward, so nothing was pushed"
    );
    let tag_ref = format!("refs/tags/{}", plan.tag);
    let tagged = run_git(
        repo,
        &["rev-parse", "--verify", &format!("{tag_ref}^{{commit}}")],
    )
    .ok();
    if let Some(at) = &tagged {
        anyhow::ensure!(
            *at == commit,
            "tag {} already exists, on {at}, not {commit}",
            plan.tag
        );
    }
    if plan.dry_run {
        return Ok(Landed {
            commit,
            tag: plan.tag.clone(),
            transport,
        });
    }
    let transport = git::push(repo, &[format!("{commit}:refs/heads/main")])?;
    if tagged.is_none() {
        let message = format!("Release {} ({})", plan.release, plan.version);
        run_git(repo, &["tag", "-a", &plan.tag, &commit, "-m", &message])?;
    }
    git::push(repo, &[format!("{tag_ref}:{tag_ref}")])?;
    Ok(Landed {
        commit,
        tag: plan.tag.clone(),
        transport,
    })
}

#[cfg(test)]
#[path = "land_tests.rs"]
mod tests;
