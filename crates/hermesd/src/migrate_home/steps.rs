//! The filesystem half of the home migration. Every change is recorded in the
//! state file the moment it happens, which is what makes resume and rollback
//! possible.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use super::state::{Action, State};
use super::Plan;
use crate::activity::claude_project_key;
use crate::brand::{daemon_file, legacy_daemon_file};

/// The daemon's own files that carry the old name: `(old, new)`, relative to
/// the home.
pub fn renamed_files() -> Vec<(PathBuf, PathBuf)> {
    let mut files: Vec<(PathBuf, PathBuf)> = [".toml", ".port", ".reclaims"]
        .iter()
        .map(|s| (legacy_daemon_file(s).into(), daemon_file(s).into()))
        .collect();
    for suffix in [".out.log", ".err.log"] {
        files.push((
            Path::new("logs").join(legacy_daemon_file(suffix)),
            Path::new("logs").join(daemon_file(suffix)),
        ));
    }
    files
}

/// Moves `from` to `to` and records it. A destination that already exists is
/// never overwritten.
pub fn rename(state: &mut State, from: &Path, to: &Path) -> anyhow::Result<()> {
    if to.symlink_metadata().is_ok() {
        bail!("{} already exists; not overwriting it", to.display());
    }
    std::fs::rename(from, to)
        .with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
    state.record(Action::Rename {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
    })
}

/// `pre-rename-<timestamp>` under the old home's `backups/`: the database (a
/// consistent copy via `VACUUM INTO`), the config, and every transcript
/// directory the migration will rename. Secrets are not copied; they are only
/// moved, never changed.
pub fn backup(plan: &Plan, state: &mut State) -> anyhow::Result<PathBuf> {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let backups = plan.from.join("backups");
    // Never reuse a directory: a retried or repeated run gets its own.
    let dir = (0..)
        .map(|n| match n {
            0 => backups.join(format!("pre-rename-{stamp}")),
            n => backups.join(format!("pre-rename-{stamp}-{n}")),
        })
        .find(|dir| !dir.exists())
        .context("no free backup directory name")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let db = plan.from.join("bus.sqlite");
    if db.is_file() {
        let conn = rusqlite::Connection::open(&db)?;
        conn.execute(
            "VACUUM INTO ?1",
            [dir.join("bus.sqlite").to_string_lossy().as_ref()],
        )
        .context("copying the database")?;
    }
    let config = plan.from.join(legacy_daemon_file(".toml"));
    if config.is_file() {
        std::fs::copy(&config, dir.join(legacy_daemon_file(".toml")))?;
    }
    let transcripts = dir.join("claude-projects");
    for (old, _) in transcript_dirs(plan)? {
        let name = old.file_name().context("transcript dir has no name")?;
        copy_tree(&old, &transcripts.join(name))?;
    }
    state.backup = Some(dir.clone());
    Ok(dir)
}

fn copy_tree(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Bytes under `path`, following no links.
pub fn tree_size(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| tree_size(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

/// Claude Code's directories for anything that ran under the old home's
/// `projects/`, paired with their names under the new home. Keyed by the same
/// mangling the chat and activity readers use, under both the home as given
/// and as resolved through symlinks.
pub fn transcript_dirs(plan: &Plan) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    let root = crate::activity::claude_projects_dir(&plan.user_home);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let prefixes: Vec<(String, String)> = plan
        .path_pairs()
        .into_iter()
        .map(|(from, to)| {
            (
                claude_project_key(&Path::new(&from).join("projects")),
                claude_project_key(&Path::new(&to).join("projects")),
            )
        })
        .collect();
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let renamed = prefixes.iter().find_map(|(old, new)| {
            let rest = name.strip_prefix(old.as_str())?;
            (rest.is_empty() || rest.starts_with('-')).then(|| format!("{new}{rest}"))
        });
        if let Some(renamed) = renamed {
            dirs.push((entry.path(), root.join(renamed)));
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// Renames each transcript directory. When the new name is already taken (a
/// session started under the new path before the migration finished), the
/// old directory's entries are moved into it one by one and anything present
/// in both is left where it was, with a warning.
pub fn move_transcripts(plan: &Plan, state: &mut State) -> anyhow::Result<usize> {
    let mut moved = 0;
    for (old, new) in transcript_dirs(plan)? {
        if new.symlink_metadata().is_err() {
            rename(state, &old, &new)?;
            moved += 1;
            continue;
        }
        for entry in std::fs::read_dir(&old)?.flatten() {
            let target = new.join(entry.file_name());
            if target.symlink_metadata().is_ok() {
                state.warn(format!(
                    "kept {}: {} already exists",
                    entry.path().display(),
                    target.display()
                ));
                continue;
            }
            rename(state, &entry.path(), &target)?;
        }
        // Empty now unless something was kept; either way nothing is lost.
        let _ = std::fs::remove_dir(&old);
        moved += 1;
    }
    Ok(moved)
}

/// Leaves `from` pointing at `to`, so paths written down before the move
/// (bot memory, scripts, the previous release on rollback) still resolve.
pub fn symlink(plan: &Plan, state: &mut State) -> anyhow::Result<()> {
    if plan.from.symlink_metadata().is_ok() {
        state.warn(format!(
            "{} exists; no compatibility link made",
            plan.from.display()
        ));
        return Ok(());
    }
    make_link(&plan.to, &plan.from)?;
    state.record(Action::Symlink {
        path: plan.from.clone(),
    })
}

#[cfg(unix)]
fn make_link(target: &Path, link: &Path) -> anyhow::Result<()> {
    std::os::unix::fs::symlink(target, link)
        .with_context(|| format!("linking {} to {}", link.display(), target.display()))
}

/// A directory junction: unlike a symlink it needs neither administrator
/// rights nor developer mode.
#[cfg(windows)]
fn make_link(target: &Path, link: &Path) -> anyhow::Result<()> {
    let out = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .context("running mklink")?;
    anyhow::ensure!(
        out.status.success(),
        "mklink /J failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

pub fn remove_link(path: &Path) -> anyhow::Result<()> {
    let Ok(meta) = path.symlink_metadata() else {
        return Ok(());
    };
    // A junction reports as a directory; removing it never touches the target.
    if meta.file_type().is_symlink() || cfg!(windows) {
        if std::fs::remove_file(path).is_err() {
            std::fs::remove_dir(path)?;
        }
        return Ok(());
    }
    bail!("{} is not a link; leaving it", path.display())
}

/// Git worktrees under the moved home record their old absolute path in the
/// main repository. `git worktree repair`, run from the worktree, rewrites
/// that link; failures are warnings, since the compatibility symlink keeps
/// the old path working.
pub fn repair_worktrees(plan: &Plan, state: &mut State) -> usize {
    let mut found = Vec::new();
    find_worktrees(&plan.to.join("projects"), 0, &mut found);
    let mut repaired = 0;
    for dir in found {
        let out = std::process::Command::new("git")
            .args(["worktree", "repair"])
            .current_dir(&dir)
            .output();
        match out {
            Ok(out) if out.status.success() => repaired += 1,
            Ok(out) => state.warn(format!(
                "git worktree repair in {}: {}",
                dir.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(error) => state.warn(format!("git worktree repair in {}: {error}", dir.display())),
        }
    }
    repaired
}

/// Linked worktrees have a `.git` *file*; bot workspaces sit five levels
/// below `projects/`, and their worktrees a little deeper.
fn find_worktrees(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    if dir.join(".git").is_file() {
        found.push(dir.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == ".git" || name == "node_modules" || name == "target" {
            continue;
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            find_worktrees(&entry.path(), depth + 1, found);
        }
    }
}

/// Free bytes on the filesystem holding `path`, where that can be asked.
#[cfg(unix)]
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `stat` a writable struct.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut stat) };
    (rc == 0).then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
}

#[cfg(windows)]
pub fn free_bytes(_path: &Path) -> Option<u64> {
    None
}
