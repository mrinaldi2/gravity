//! `hermesd release build-installer <release> [--commit <sha>]
//! [--script <path>] [--output <file>] [--timeout <minutes>]` (H-117 X3,
//! H-104): Tester Win builds the Windows installer with no owner prompt.
//!
//! The daemon checks first (`hermes/release_build_installer`): the
//! `build_installers` extra and a release the owner approved. Then, in the
//! bot's release worktree:
//! - HEAD is the release's commit (the release branch's tip unless named),
//!   and nothing is changed or untracked (`git status --porcelain`);
//! - the script (`scripts/build-nsis.ps1` unless named) is byte for byte
//!   the one committed there (its blob hash);
//! - it runs with `powershell -NoProfile -ExecutionPolicy Bypass -File`,
//!   with a time limit;
//! - the installer it made is hashed and recorded on the release, for
//!   `release publish` (`hermes/release_installer_built`).
//!
//! Only this command is allowed, never the script itself: the bot can edit
//! the script in its worktree, so a rule for it would run anything.
//! Residual: the tree is checked, then run; a bot racing its own worktree
//! in between is the same-user limit every guard here has.

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
    let commit = match &args.commit {
        Some(commit) => commit.clone(),
        None => {
            git::fetch(&repo, &[&branch])?;
            format!("refs/remotes/origin/{branch}")
        }
    };
    let script = args.script.as_deref().unwrap_or(SCRIPT);
    let commit = check_tree(&repo, &commit, script)?;
    println!("{script} is as committed at {commit}; building");
    let since = SystemTime::now();
    let limit = Duration::from_secs(60 * args.timeout_minutes.unwrap_or(TIMEOUT_MINUTES));
    run_script(&repo, script, limit)?;
    let file = made_since(&repo, args.output.as_deref(), since)?;
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
