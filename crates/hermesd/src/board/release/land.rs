//! `hermesd release land <release> [--commit <sha>] [--branch <b>]
//! [--tag <t>] [--dry-run]` (H-117 X2): DevOps puts an approved release on
//! `main` and tags it, with no owner step.
//!
//! The daemon checks first (`hermes/release_land`): the `release_main`
//! extra, the project's DevOps role, a release the owner approved, and the
//! one commit its builds were made from (ARCH-R52 M1). Then, in DevOps's own
//! checkout, with git hooks off:
//! - that commit and nothing else lands (`--commit` may only restate it);
//! - the release branch (`release/desktop-<version>` unless named) must still
//!   be at it, so code pushed after the builds can never land;
//! - `main` must fast-forward to it: never forced, a diverged main is
//!   refused;
//! - `main` is pushed, then the annotated tag (`desktop-v<version>` unless
//!   named). The project's repo is then asked directly that both are at the
//!   commit before they go on the release (`hermes/release_landed`).
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
    /// The commit the release's builds were made from.
    pub commit: String,
    pub branch: String,
    pub tag: String,
    pub dry_run: bool,
    /// Where the result is checked: the project's configured repo, else
    /// the checkout's origin.
    pub repo_url: Option<String>,
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
    /// The plan, for the commit the daemon recorded, with the repo's own
    /// conventions for what isn't named.
    pub(crate) fn plan(
        self,
        version: &str,
        recorded: &str,
        repo_url: Option<String>,
    ) -> anyhow::Result<Plan> {
        if let Some(named) = &self.commit {
            anyhow::ensure!(
                named == recorded,
                "release {} was built from {recorded}; --commit can only name that",
                self.release
            );
        }
        Ok(Plan {
            branch: self
                .branch
                .unwrap_or_else(|| format!("release/desktop-{version}")),
            tag: self.tag.unwrap_or_else(|| format!("desktop-v{version}")),
            release: self.release,
            version: version.to_string(),
            commit: recorded.to_string(),
            dry_run: self.dry_run,
            repo_url,
        })
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
    let recorded = gate["commit"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("the daemon named no commit for this release"))?;
    let repo_url = gate["repo_url"].as_str().map(str::to_string);
    let plan = args.plan(&version, recorded, repo_url)?;
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
    let commit = plan.commit.clone();
    let tip = run_git(
        repo,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/remotes/origin/{}^{{commit}}", plan.branch),
        ],
    )
    .map_err(|_| anyhow::anyhow!("no branch {} on origin", plan.branch))?;
    anyhow::ensure!(
        tip == commit,
        "{} is at {tip}, but release {} was built from {commit}: something was pushed after \
         the builds, so it can't land; package a new release",
        plan.branch,
        plan.release
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
    verify_remote(repo, plan, &commit, &tag_ref)?;
    Ok(Landed {
        commit,
        tag: plan.tag.clone(),
        transport,
    })
}

/// The repo itself, asked directly: main and the tag are at the commit.
fn verify_remote(repo: &Path, plan: &Plan, commit: &str, tag_ref: &str) -> anyhow::Result<()> {
    let url = match &plan.repo_url {
        Some(url) => url.clone(),
        None => run_git(repo, &["remote", "get-url", "origin"])?,
    };
    let peeled = format!("{tag_ref}^{{}}");
    let refs = git::remote_refs(&url, &["refs/heads/main".to_string(), peeled.clone()])?;
    let at = |name: &str| {
        refs.iter()
            .find(|(n, _)| n == name)
            .map(|(_, sha)| sha.as_str())
    };
    anyhow::ensure!(
        at("refs/heads/main") == Some(commit),
        "pushed, but {url} has main at {}, not {commit}; nothing recorded",
        at("refs/heads/main").unwrap_or("nothing")
    );
    anyhow::ensure!(
        at(&peeled) == Some(commit),
        "pushed, but {url} has no tag {} on {commit}; nothing recorded",
        plan.tag
    );
    Ok(())
}

#[cfg(test)]
#[path = "land_tests.rs"]
mod tests;
