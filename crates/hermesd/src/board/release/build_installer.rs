//! `hermesd release build-installer <release> [--commit <sha>]
//! [--script <path>] [--output <file>] [--timeout <minutes>]` (H-117 X3,
//! H-104): Tester Win builds the Windows installer with no owner prompt.
//!
//! The daemon checks first (`hermes/release_build_installer`): the
//! `build_installers` extra and a release still open for builds, and names
//! the commit its builds were made from, if any yet (ARCH-R52 M1). Then:
//! - the commit is that one (`--commit` may only restate it), else the
//!   release branch's tip;
//! - a fresh detached worktree of it is made in a private temp folder, with
//!   git hooks off, so nothing the bot changed, marked unchanged or left
//!   ignored in its own worktree takes part (ARCH-R52 S2);
//! - there, the tree is clean, no file is marked assume-unchanged or
//!   skip-worktree, and the script (`scripts/build-nsis.ps1` unless named)
//!   is its committed blob;
//! - it runs with `powershell -NoProfile -ExecutionPolicy Bypass -File`,
//!   with a time limit;
//! - the installer is copied to `<checkout>/target/release-installers/`,
//!   hashed and recorded on the release with the commit
//!   (`hermes/release_installer_built`), and the temp worktree removed.
//!
//! Only this command is allowed, never the script itself: the bot can edit
//! the script in its worktree, so a rule for it would run anything.
//! Residual: the temp worktree is the same user's, so a bot racing it during
//! the build is the same-user limit every guard here has.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use serde_json::json;

use super::git::{self, git as run_git};
use super::land::ask;
use crate::config::Config;

const USAGE: &str = "usage: hermesd release build-installer <release> [--commit <sha>] \
                     [--script <path>] [--output <file>] [--timeout <minutes>]";

const SCRIPT: &str = "scripts/build-nsis.ps1";
const TIMEOUT_MINUTES: u64 = 45;

/// Where Tauri's NSIS bundler leaves the setup, under the repo.
const NSIS_DIRS: [&str; 2] = [
    "apps/desktop/src-tauri/target/release/bundle/nsis",
    "target/release/bundle/nsis",
];

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Args {
    pub release: String,
    pub commit: Option<String>,
    pub script: Option<String>,
    pub output: Option<PathBuf>,
    pub timeout_minutes: Option<u64>,
}

pub(crate) fn parse(args: &[String]) -> anyhow::Result<Args> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--commit" => out.commit = it.next().cloned(),
            "--script" => out.script = it.next().cloned(),
            "--output" => out.output = it.next().map(PathBuf::from),
            "--timeout" => out.timeout_minutes = it.next().and_then(|m| m.parse().ok()),
            a if a.starts_with("--") || !out.release.is_empty() => anyhow::bail!("{USAGE}"),
            a => out.release = a.to_string(),
        }
    }
    anyhow::ensure!(!out.release.is_empty(), "{USAGE}");
    Ok(out)
}

/// The worktree is exactly the release's commit, and the script exactly as
/// committed there. Returns the commit.
pub fn check_tree(repo: &Path, commit: &str, script: &str) -> anyhow::Result<String> {
    let commit = run_git(
        repo,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )
    .map_err(|_| anyhow::anyhow!("no commit {commit} here; fetch the release branch first"))?;
    let head = run_git(repo, &["rev-parse", "HEAD"])?;
    anyhow::ensure!(
        head == commit,
        "this worktree is at {head}, not the release's {commit}: check out the release first"
    );
    let dirty = run_git(repo, &["status", "--porcelain"])?;
    anyhow::ensure!(
        dirty.is_empty(),
        "this worktree has changes, so it isn't the release as committed:\n{dirty}"
    );
    // Files status can't see: marked assume-unchanged (lowercase tag) or
    // skip-worktree (`S`).
    let flagged: Vec<String> = run_git(repo, &["ls-files", "-v"])?
        .lines()
        .filter(|l| l.starts_with(|c: char| c.is_ascii_lowercase() || c == 'S'))
        .map(str::to_string)
        .collect();
    anyhow::ensure!(
        flagged.is_empty(),
        "files are marked so git doesn't see their changes:\n{}",
        flagged.join("\n")
    );
    let committed = run_git(repo, &["rev-parse", &format!("{commit}:{script}")])
        .map_err(|_| anyhow::anyhow!("{script} isn't committed at {commit}"))?;
    let on_disk = run_git(repo, &["hash-object", "--", script])?;
    anyhow::ensure!(
        on_disk == committed,
        "{script} differs from the one committed at {commit}; it runs only as committed"
    );
    Ok(commit)
}

/// The newest setup the build left, or the one named.
fn made_since(repo: &Path, named: Option<&Path>, since: SystemTime) -> anyhow::Result<PathBuf> {
    if let Some(file) = named {
        let file = if file.is_absolute() {
            file.to_path_buf()
        } else {
            repo.join(file)
        };
        anyhow::ensure!(file.is_file(), "no installer at {}", file.display());
        return Ok(file);
    }
    NSIS_DIRS
        .iter()
        .filter_map(|dir| std::fs::read_dir(repo.join(dir)).ok())
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with("-setup.exe"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .filter(|(at, _)| *at >= since)
        .max_by_key(|(at, _)| *at)
        .map(|(_, p)| p)
        .ok_or_else(|| anyhow::anyhow!("the build made no *-setup.exe; name it with --output"))
}

/// Runs the script with a time limit; its output goes to ours.
fn run_script(repo: &Path, script: &str, limit: Duration) -> anyhow::Result<()> {
    let mut child = Command::new("powershell")
        .current_dir(repo)
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script])
        .spawn()
        .map_err(|e| anyhow::anyhow!("can't start powershell: {e}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(status.success(), "{script} exited with {status}");
            return Ok(());
        }
        if started.elapsed() >= limit {
            let _ = child.kill();
            anyhow::bail!(
                "{script} ran past its {} minutes; stopped",
                limit.as_secs() / 60
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// A detached worktree of one commit in a private temp folder, removed
/// with its build when dropped.
pub struct Fresh {
    repo: PathBuf,
    pub dir: PathBuf,
}

impl Fresh {
    pub fn add(repo: &Path, commit: &str) -> anyhow::Result<Fresh> {
        let dir = std::env::temp_dir().join(format!("hermes-installer-{}", bus::new_id()));
        let mut args: Vec<&str> = git::no_hooks().to_vec();
        let path = dir.display().to_string();
        args.extend(["worktree", "add", "--detach", &path, commit]);
        run_git(repo, &args)?;
        Ok(Fresh {
            repo: repo.to_path_buf(),
            dir,
        })
    }
}

impl Drop for Fresh {
    fn drop(&mut self) {
        let path = self.dir.display().to_string();
        let _ = run_git(&self.repo, &["worktree", "remove", "--force", &path]);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The commit to build: the one the release's builds name (only restated
/// by `--commit`), else the one named, else the release branch's tip.
fn commit_to_build(
    repo: &Path,
    recorded: Option<&str>,
    named: Option<&str>,
    branch: &str,
) -> anyhow::Result<String> {
    if let (Some(recorded), Some(named)) = (recorded, named) {
        anyhow::ensure!(
            recorded == named,
            "this release's builds come from {recorded}; --commit can only name that"
        );
    }
    git::fetch(repo, &[branch])?;
    let wanted = recorded
        .or(named)
        .map_or_else(|| format!("refs/remotes/origin/{branch}"), str::to_string);
    run_git(
        repo,
        &["rev-parse", "--verify", &format!("{wanted}^{{commit}}")],
    )
    .map_err(|_| anyhow::anyhow!("no commit {wanted} here"))
}

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let args = parse(args)?;
    let gate = ask(
        cfg,
        "hermes/release_build_installer",
        json!({ "release_id": args.release }),
    )
    .await?;
    let version = gate["version"].as_str().unwrap_or_default();
    let repo = git::toplevel(&std::env::current_dir()?)?;
    let branch = format!("release/desktop-{version}");
    let commit = commit_to_build(
        &repo,
        gate["commit"].as_str(),
        args.commit.as_deref(),
        &branch,
    )?;
    let script = args.script.as_deref().unwrap_or(SCRIPT);
    let fresh = Fresh::add(&repo, &commit)?;
    check_tree(&fresh.dir, &commit, script)?;
    println!(
        "{script} is as committed at {commit}; building in {}",
        fresh.dir.display()
    );
    let since = SystemTime::now();
    let limit = Duration::from_secs(60 * args.timeout_minutes.unwrap_or(TIMEOUT_MINUTES));
    run_script(&fresh.dir, script, limit)?;
    let built = made_since(&fresh.dir, args.output.as_deref(), since)?;
    let kept = repo.join("target").join("release-installers");
    std::fs::create_dir_all(&kept)?;
    let file = kept.join(built.file_name().unwrap_or_default());
    std::fs::copy(&built, &file)?;
    drop(fresh);
    let sha256 = crate::quiesce::file_sha256(&file)?;
    let file = file.display().to_string();
    ask(
        cfg,
        "hermes/release_installer_built",
        json!({ "release_id": args.release, "commit": commit, "file": file, "sha256": sha256 }),
    )
    .await?;
    println!(
        "built {file}\nsha256 {sha256}\nnext: hermesd release publish {} <file>",
        args.release
    );
    Ok(())
}

#[cfg(test)]
#[path = "build_installer_tests.rs"]
mod tests;
