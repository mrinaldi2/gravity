//! Git for the release commands (H-117 X2, X3), in the bot's own checkout.
//! A push or fetch tries the remote as configured and, when SSH can't
//! authenticate (an agent without a key), GitHub over HTTPS with the gh
//! CLI's credential, for that one command: no git config changes. Nothing
//! here ever forces.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `git <args>` in `repo`: its trimmed stdout, or its stderr as the error.
pub fn git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git").current_dir(repo).args(args).output()?;
    anyhow::ensure!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Whether `git <args>` succeeds (a yes/no question to git).
pub fn yes(repo: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The top of the checkout the command runs in.
pub fn toplevel(dir: &Path) -> anyhow::Result<PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .map_err(|_| anyhow::anyhow!("run this from your checkout of the project's repo"))
}

/// `git@github.com:o/r.git` (or an https URL) as GitHub's https URL.
pub(crate) fn github_https(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))?;
    Some(format!("https://github.com/{path}"))
}

/// SSH refused for want of a key: worth one try over HTTPS.
pub(crate) fn ssh_refused(stderr: &str) -> bool {
    [
        "Permission denied (publickey)",
        "Could not read from remote repository",
        "Host key verification failed",
    ]
    .iter()
    .any(|s| stderr.contains(s))
}

/// No hooks: a pre-push hook in the bot's checkout would otherwise run
/// inside the gated command (ARCH-R52 S1).
pub(crate) fn no_hooks() -> [&'static str; 2] {
    [
        "-c",
        if cfg!(windows) {
            "core.hooksPath=NUL"
        } else {
            "core.hooksPath=/dev/null"
        },
    ]
}

/// `git <verb> origin <refspecs>`, falling back to GitHub over HTTPS with
/// gh's credential. Returns how it went out.
fn to_origin(repo: &Path, verb: &str, refspecs: &[String]) -> anyhow::Result<&'static str> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(no_hooks())
        .arg(verb)
        .arg("origin")
        .args(refspecs)
        .output()?;
    if out.status.success() {
        return Ok("origin");
    }
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let url = git(repo, &["remote", "get-url", "origin"]).unwrap_or_default();
    let https = github_https(&url).filter(|_| ssh_refused(&stderr));
    let Some(https) = https else {
        anyhow::bail!("git {verb}: {}", stderr.trim());
    };
    let retry = Command::new("git")
        .current_dir(repo)
        .args(no_hooks())
        .args([
            "-c",
            "credential.helper=",
            "-c",
            "credential.helper=!gh auth git-credential",
        ])
        .arg(verb)
        .arg(&https)
        .args(refspecs)
        .output()?;
    anyhow::ensure!(
        retry.status.success(),
        "git {verb} (SSH had no key, and HTTPS with gh's credential failed too): {}",
        String::from_utf8_lossy(&retry.stderr).trim()
    );
    Ok("https (gh)")
}

/// Fetches `branches` from origin into `refs/remotes/origin/*`, and its tags.
pub fn fetch(repo: &Path, branches: &[&str]) -> anyhow::Result<&'static str> {
    let mut refspecs: Vec<String> = branches
        .iter()
        .map(|b| format!("+refs/heads/{b}:refs/remotes/origin/{b}"))
        .collect();
    refspecs.push("+refs/tags/*:refs/tags/*".to_string());
    to_origin(repo, "fetch", &refspecs)
}

/// Pushes `refspecs` to origin, never forced.
pub fn push(repo: &Path, refspecs: &[String]) -> anyhow::Result<&'static str> {
    anyhow::ensure!(
        refspecs.iter().all(|r| !r.starts_with('+')),
        "a release push is never forced"
    );
    to_origin(repo, "push", refspecs)
}

/// Deletes `branch` on origin only while its tip is still `expect`: the
/// lease makes the check and the delete one step on the remote, so a branch
/// pushed to after the merge is kept (H-284, H-261 §15.2).
pub fn delete_branch(repo: &Path, branch: &str, expect: &str) -> anyhow::Result<&'static str> {
    to_origin(
        repo,
        "push",
        &[
            format!("--force-with-lease=refs/heads/{branch}:{expect}"),
            format!(":refs/heads/{branch}"),
        ],
    )
}

/// What `url` has for each of `refs` (`refs/tags/x^{}` for a tag's commit),
/// asked outside the checkout and with no git config of the user's or the
/// checkout's, so no `insteadOf` or `pushurl` can redirect it (ARCH-R52 S1).
pub fn remote_refs(url: &str, refs: &[String]) -> anyhow::Result<Vec<(String, String)>> {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let url = github_https(url).unwrap_or_else(|| url.to_string());
    let out = Command::new("git")
        .current_dir(std::env::temp_dir())
        .env("GIT_CONFIG_GLOBAL", null)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(no_hooks())
        .args(["-c", "credential.helper=!gh auth git-credential"])
        .args(["ls-remote", &url])
        .args(refs)
        .output()?;
    anyhow::ensure!(
        out.status.success(),
        "git ls-remote {url}: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (sha, name) = l.split_once('\t')?;
            Some((name.to_string(), sha.to_string()))
        })
        .collect())
}
