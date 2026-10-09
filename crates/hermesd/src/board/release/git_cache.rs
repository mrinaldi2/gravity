//! The daemon's own copy of a project's history (H-121), to tell whether one
//! commit contains another without trusting any bot's checkout: a bare,
//! blob-less clone under `<home>/cache/repos/<project>.git`, cloned and
//! fetched with hooks off and no git config of the user's (ARCH-R52 S1).
//! Bots fill it by pushing, so every git here goes through the daemon's
//! hardened runner (H-289).

use std::path::{Path, PathBuf};

use super::git::github_https;
use crate::safe_git::SafeGit;

/// The cache for a project.
pub fn dir(home: &Path, project_id: &str) -> PathBuf {
    home.join("cache")
        .join("repos")
        .join(format!("{project_id}.git"))
}

/// git in `cwd`, allowed to fetch from the project's repository.
fn git(cwd: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    SafeGit::fetching(cwd)?.args(args).output()
}

fn ok(out: &std::process::Output, what: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        out.status.success(),
        "git {what}: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

/// Clones the project's repository into the cache, or fetches every branch
/// and tag into it.
pub fn refresh(home: &Path, project_id: &str, url: &str) -> anyhow::Result<PathBuf> {
    let url = github_https(url).unwrap_or_else(|| url.to_string());
    let cache = dir(home, project_id);
    if cache.join("HEAD").exists() {
        let out = git(
            &cache,
            &[
                "fetch",
                "--quiet",
                "--prune",
                "--filter=blob:none",
                &url,
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ],
        )?;
        ok(&out, "fetch")?;
    } else {
        let parent = cache.parent().expect("cache has a parent");
        std::fs::create_dir_all(parent)?;
        let target = cache.display().to_string();
        let out = git(
            parent,
            &[
                "clone",
                "--quiet",
                "--bare",
                "--filter=blob:none",
                &url,
                &target,
            ],
        )?;
        ok(&out, "clone")?;
    }
    Ok(cache)
}

/// Whether the cache has `commit`.
pub fn has(cache: &Path, commit: &str) -> bool {
    git(cache, &["cat-file", "-e", &format!("{commit}^{{commit}}")])
        .is_ok_and(|o| o.status.success())
}

/// Whether `ancestor` is `descendant` or in its history.
pub fn contains(cache: &Path, descendant: &str, ancestor: &str) -> anyhow::Result<bool> {
    let out = git(
        cache,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => {
            ok(&out, "merge-base")?;
            Ok(false)
        }
    }
}

/// The commit a ref names in the cache, if it names one.
pub fn resolve(cache: &Path, reference: &str) -> Option<String> {
    let out = git(
        cache,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{reference}^{{commit}}"),
        ],
    )
    .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|c| !c.is_empty())
}

/// When `commit` was committed (its committer date), if the cache has it.
pub fn commit_time(cache: &Path, commit: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let out = git(
        cache,
        &[
            "show",
            "-s",
            "--format=%ct",
            &format!("{commit}^{{commit}}"),
        ],
    )
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let secs = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    chrono::DateTime::from_timestamp(secs, 0)
}

/// The best common ancestor of `a` and `b`.
pub fn merge_base(cache: &Path, a: &str, b: &str) -> anyhow::Result<String> {
    let out = git(cache, &["merge-base", a, b])?;
    ok(&out, "merge-base")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `git patch-id --stable` of the change from `base` to `head` (H-261 §3):
/// the same for the same change whatever it was rebased onto; empty when
/// there is no change. A blob-less cache fetches the blobs it needs.
pub fn patch_id(cache: &Path, base: &str, head: &str) -> anyhow::Result<String> {
    let diff = git(
        cache,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--full-index",
            base,
            head,
        ],
    )?;
    ok(&diff, "diff")?;
    if diff.stdout.is_empty() {
        return Ok(String::new());
    }
    let out = SafeGit::local(cache)?
        .args(&["patch-id", "--stable"])
        .output_with_stdin(diff.stdout)?;
    ok(&out, "patch-id")?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string())
}
