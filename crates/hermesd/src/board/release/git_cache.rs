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

/// git in the cache. A blob-less cache may fetch what it lacks from the
/// repository it was cloned from, so this may fetch from there too.
fn git(cache: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    let origin = SafeGit::local(cache)?
        .args(&["config", "--get", "remote.origin.url"])
        .run()
        .unwrap_or_default();
    fetch(cache, &origin, args)
}

/// git in `cwd`, allowed to fetch from `url`, the project's repository.
fn fetch(cwd: &Path, url: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
    SafeGit::fetching(cwd, url)?.args(args).output()
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
        let out = fetch(
            &cache,
            &url,
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
        let out = fetch(
            parent,
            &url,
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
    resolve_kind(cache, reference, "commit")
}

fn resolve_kind(cache: &Path, reference: &str, kind: &str) -> Option<String> {
    let out = git(
        cache,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{reference}^{{{kind}}}"),
        ],
    )
    .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|c| !c.is_empty())
}

/// The tree `commit` records, if the cache has it: two commits with the
/// same tree hold the same files (H-261 §1.5).
pub fn tree_of(cache: &Path, commit: &str) -> Option<String> {
    resolve_kind(cache, commit, "tree")
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

/// The text of `path` at `commit`, or `None` when that tree has no such
/// file. Its blob is fetched on demand, the cache being blob-less.
pub fn file_at(cache: &Path, commit: &str, path: &str) -> anyhow::Result<Option<String>> {
    let out = git(cache, &["ls-tree", "--name-only", commit, "--", path])?;
    ok(&out, "ls-tree")?;
    if String::from_utf8_lossy(&out.stdout).trim() != path {
        return Ok(None);
    }
    let out = git(cache, &["cat-file", "blob", &format!("{commit}:{path}")])?;
    ok(&out, "cat-file")?;
    Ok(Some(String::from_utf8(out.stdout)?))
}

/// The paths `head` changes since it left `base` (their merge base). A
/// rename counts as both its paths; no blobs are needed.
pub fn changed_paths(cache: &Path, base: &str, head: &str) -> anyhow::Result<Vec<String>> {
    let range = format!("{base}...{head}");
    let out = git(
        cache,
        &["diff", "--name-only", "--no-renames", "-z", &range, "--"],
    )?;
    ok(&out, "diff")?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// The files merging `a` and `b` would conflict on, or empty when it merges
/// cleanly (`git merge-tree --write-tree`, writing nothing to any ref).
pub fn conflicts(cache: &Path, a: &str, b: &str) -> anyhow::Result<Vec<String>> {
    let out = git(
        cache,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            a,
            b,
        ],
    )?;
    match out.status.code() {
        Some(0) => Ok(Vec::new()),
        // The first line is the tree written; the conflicted files follow.
        Some(1) => Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .skip(1)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()),
        _ => {
            ok(&out, "merge-tree")?;
            Ok(Vec::new())
        }
    }
}

/// One hunk of a zero-context diff: old lines `old_start..old_start+old_len`
/// became `new_len` lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_len: u32,
}

/// The hunks that turn `path` at `from` into `path` at `to`, or `None` when
/// `to` has no such file. No textconv or external diff runs (SafeGit).
pub fn hunks(cache: &Path, from: &str, to: &str, path: &str) -> anyhow::Result<Option<Vec<Hunk>>> {
    let listed = git(cache, &["ls-tree", "--name-only", to, "--", path])?;
    ok(&listed, "ls-tree")?;
    if String::from_utf8_lossy(&listed.stdout).trim() != path {
        return Ok(None);
    }
    let out = git(
        cache,
        &[
            "diff",
            "-U0",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            from,
            to,
            "--",
            path,
        ],
    )?;
    ok(&out, "diff")?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(Some(text.lines().filter_map(parse_hunk).collect()))
}

/// `@@ -a[,b] +c[,d] @@`, where a missing count is 1.
fn parse_hunk(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let new = rest.split_once(" @@")?.0;
    let range = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (old_start, old_len) = range(old)?;
    let (_, new_len) = range(new)?;
    Some(Hunk {
        old_start,
        old_len,
        new_len,
    })
}
