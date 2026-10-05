//! Keeping what a retired worker's repositories hold that no remote has,
//! before the daemon deletes them (ARCH-R40). Saving to the worker's branch
//! covers only its `repo/` clone and can fail, so every top-level repository
//! in `repo/` and `scratch/` with a local ref no remote has is written to
//! `<workspace>/salvage/<name>.bundle` first. A repository that can't be
//! bundled is kept where it is.

use std::path::{Path, PathBuf};

use super::git::{git, LOCAL_TIMEOUT};

/// Where bundles go, relative to the workspace. It stays until retention.
pub const SALVAGE_DIR: &str = "salvage";

/// The git repositories at the top of `dir`: `dir` itself if it is one,
/// else each folder directly in it that is.
pub(super) fn repos_in(dir: &Path) -> Vec<PathBuf> {
    if dir.join(".git").exists() {
        return vec![dir.to_path_buf()];
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| path.join(".git").exists())
        .collect()
}

/// The bundle a repository is saved to: its path in the workspace, `/`s
/// made `-`s, so `scratch/other` is `salvage/scratch-other.bundle`.
pub fn bundle_path(workspace: &Path, repo: &Path) -> PathBuf {
    let name = repo
        .strip_prefix(workspace)
        .unwrap_or(repo)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("-");
    workspace.join(SALVAGE_DIR).join(format!("{name}.bundle"))
}

/// Writes the commits `repo` has that no remote has to its bundle. `None`
/// when there are none, or an earlier pass already wrote it; an error when
/// they couldn't be saved, and the repository must stay.
pub fn save(workspace: &Path, repo: &Path) -> anyhow::Result<Option<PathBuf>> {
    let bundle = bundle_path(workspace, repo);
    if bundle.exists() {
        return Ok(None);
    }
    let Some(revs) = unpushed(repo)? else {
        return Ok(None);
    };
    if let Some(dir) = bundle.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let path = bundle.display().to_string();
    let mut args = vec!["bundle", "create", path.as_str()];
    args.extend(revs);
    in_repo(repo, &args)?;
    anyhow::ensure!(bundle.is_file(), "git wrote no bundle at {path}");
    Ok(Some(bundle))
}

/// The revisions to bundle, if any local commit is on no remote. A clone
/// owns its branches; a linked worktree's branches belong to a repository
/// that isn't deleted, so only a detached HEAD of its own is at risk.
fn unpushed(repo: &Path) -> anyhow::Result<Option<Vec<&'static str>>> {
    let revs: Vec<&str> = if repo.join(".git").is_dir() {
        vec!["--branches", "--not", "--remotes"]
    } else {
        if in_repo(repo, &["rev-parse", "-q", "--verify", "HEAD"]).is_err() {
            return Ok(None);
        }
        vec!["HEAD", "--not", "--remotes", "--branches"]
    };
    let mut list = vec!["rev-list", "-n", "1"];
    list.extend(&revs);
    let any = !in_repo(repo, &list)?.trim().is_empty();
    Ok(any.then_some(revs))
}

/// git on exactly `repo`'s own `.git`, so a broken one is an error rather
/// than git finding some repository above it.
fn in_repo(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let git_dir = format!("--git-dir={}", repo.join(".git").display());
    let mut full = vec![git_dir.as_str()];
    full.extend_from_slice(args);
    git(repo, &full, LOCAL_TIMEOUT)
}
