//! `hermesd release tag <release> [--dry-run]` (H-272; H-261 §6.3): DevOps
//! tags a release cut from main, after the owner's approval. Main is already
//! at the commit: only the annotated tag `desktop-v<version>` is pushed, and
//! no release branch is made.
//!
//! The daemon checks first (`hermes/release_tag`, see `tag`). Then, in
//! DevOps' own checkout, with git hooks off: origin must be the release's
//! repo, the commit must be on main, an existing tag must already be on it;
//! the tag is pushed, the repo asked for it, and the daemon records it
//! (`hermes/release_tagged`) after asking the repo itself.

use std::path::Path;

use serde_json::{json, Value};

use super::git::{self, git as run_git};
use super::land::ask;
use crate::config::Config;
use crate::prs::repo;

const USAGE: &str = "usage: hermesd release tag <release> [--dry-run]";

/// What to tag, as the daemon said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub release: String,
    pub commit: String,
    pub tag: String,
    pub repo_url: String,
    pub dry_run: bool,
}

impl Plan {
    pub fn from_gate(gate: &Value, release: &str, dry_run: bool) -> anyhow::Result<Self> {
        let text = |key: &str| {
            gate[key]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("the daemon named no {key}"))
        };
        Ok(Self {
            release: release.to_string(),
            commit: text("commit")?,
            tag: text("tag")?,
            repo_url: text("repo_url")?,
            dry_run,
        })
    }
}

fn parse(args: &[String]) -> anyhow::Result<(String, bool)> {
    let mut release = None;
    let mut dry_run = false;
    for arg in args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            a if a.starts_with("--") || release.is_some() => anyhow::bail!("{USAGE}"),
            a => release = Some(a.to_string()),
        }
    }
    Ok((release.ok_or_else(|| anyhow::anyhow!("{USAGE}"))?, dry_run))
}

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let (release, dry_run) = parse(args)?;
    let gate = ask(cfg, "hermes/release_tag", json!({ "release_id": release })).await?;
    let plan = Plan::from_gate(&gate, &release, dry_run)?;
    let checkout = git::toplevel(&std::env::current_dir()?)?;
    tag_in(&checkout, &plan)?;
    if dry_run {
        println!("dry run: would tag {} as {}", plan.commit, plan.tag);
        return Ok(());
    }
    ask(
        cfg,
        "hermes/release_tagged",
        json!({ "release_id": release, "tag": plan.tag, "commit": plan.commit }),
    )
    .await?;
    println!(
        "release {release}: tagged {} at {} on main",
        plan.tag, plan.commit
    );
    Ok(())
}

/// The git half, in `checkout`.
pub fn tag_in(checkout: &Path, plan: &Plan) -> anyhow::Result<()> {
    let origin = run_git(checkout, &["remote", "get-url", "origin"])?;
    anyhow::ensure!(
        repo::same(&origin, &plan.repo_url),
        "this checkout's origin is {origin}, not {}; run it in a checkout of that repo",
        plan.repo_url
    );
    git::fetch(checkout, &["main"])?;
    anyhow::ensure!(
        git::yes(
            checkout,
            &[
                "merge-base",
                "--is-ancestor",
                &plan.commit,
                "refs/remotes/origin/main"
            ]
        ),
        "{} isn't on main: a release is tagged on main only",
        plan.commit
    );
    let tag_ref = format!("refs/tags/{}", plan.tag);
    let tagged = run_git(
        checkout,
        &["rev-parse", "--verify", &format!("{tag_ref}^{{commit}}")],
    )
    .ok();
    if let Some(at) = &tagged {
        anyhow::ensure!(
            *at == plan.commit,
            "tag {} already exists, on {at}, not {}",
            plan.tag,
            plan.commit
        );
    }
    if plan.dry_run {
        return Ok(());
    }
    if tagged.is_none() {
        let message = format!("Release {}", plan.release);
        run_git(
            checkout,
            &["tag", "-a", &plan.tag, &plan.commit, "-m", &message],
        )?;
    }
    git::push(checkout, &[format!("{tag_ref}:{tag_ref}")])?;
    let peeled = format!("{tag_ref}^{{}}");
    let refs = git::remote_refs(&plan.repo_url, std::slice::from_ref(&peeled))?;
    let at = refs
        .iter()
        .find(|(n, _)| *n == peeled)
        .map(|(_, s)| s.as_str());
    anyhow::ensure!(
        at == Some(plan.commit.as_str()),
        "pushed, but {} has no tag {} on {}; nothing recorded",
        plan.repo_url,
        plan.tag,
        plan.commit
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn a_release_and_dry_run() {
        assert_eq!(parse(&["r1".into()]).unwrap(), ("r1".into(), false));
        assert_eq!(
            parse(&["r1".into(), "--dry-run".into()]).unwrap(),
            ("r1".into(), true)
        );
        assert!(parse(&[]).is_err());
        assert!(parse(&["r1".into(), "r2".into()]).is_err());
    }
}
