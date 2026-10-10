//! The removal itself (H-261 §15.3 rules 5–7): the worktree's build output
//! by path, then `git worktree remove` from its main clone and a prune, all
//! with hooks off through [`SafeGit`]. Never a forced remove: git's own
//! clean check stays a last line. The links are checked again before each
//! delete and before every retry.
//!
//! On Windows a file an antivirus or the indexer holds makes a delete fail
//! for a moment: it is retried with a growing wait, and a file still held
//! after the last one leaves the job held, naming the holder. Long paths go
//! to the file system in their `\\?\` spelling.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::model::Outcome;
use crate::safe_git::SafeGit;

/// Build output under a worktree, removed by path with it (§15.2).
pub const BUILD_OUTPUT: &[&str] = &["target", "node_modules", ".build", "DerivedData"];

/// Waits between attempts at a delete a held file stopped: about 16 s in
/// all, long enough for a scan of what was just written.
pub const HELD_WAITS_MS: [u64; 6] = [250, 500, 1000, 2000, 4000, 8000];

/// The bytes under `dir`, links not followed.
pub fn size_of(dir: &Path) -> u64 {
    let mut total = 0;
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if super::scope::is_link(&meta) {
                continue;
            }
            if meta.is_dir() {
                dirs.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

/// `path` as the file system takes it: the `\\?\` spelling on Windows, so
/// a path past 260 characters can still be deleted.
fn long(path: &Path) -> PathBuf {
    if cfg!(windows) {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

/// Runs `op` until it works, retrying an error a held file causes after
/// each wait in `waits`, with `recheck` run again before each retry. Still
/// held after the last: `Held`, naming whoever holds `under`. Any other
/// error: `Failed`. Nothing more is deleted once it stops.
pub fn retrying(
    under: &Path,
    waits: &[u64],
    recheck: &dyn Fn() -> Result<(), String>,
    mut op: impl FnMut() -> anyhow::Result<()>,
) -> Result<(), Outcome> {
    let mut waits = waits.iter();
    let mut tries = 0;
    loop {
        tries += 1;
        let error = match op() {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        if !crate::holders::is_held_error(&error) {
            return Err(Outcome::Failed(format!("{error:#}")));
        }
        let Some(ms) = waits.next() else {
            let who = crate::holders::list(&crate::holders::spellings(under))
                .ok()
                .filter(|h| !h.is_empty())
                .map(|h| crate::holders::held_by(&h))
                .unwrap_or_else(|| "held by a process that couldn't be named".to_string());
            return Err(Outcome::Held(format!(
                "a file in {} is still {who} after {} tries",
                under.display(),
                tries
            )));
        };
        std::thread::sleep(Duration::from_millis(*ms));
        recheck().map_err(Outcome::Held)?;
    }
}

/// `remove_dir_all`, after making read-only files writable (links skipped,
/// never followed) if the first try fails. The OS error stays in the chain,
/// so a held file can be told apart.
fn remove_dir(dir: &Path) -> anyhow::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {}
    }
    crate::workers::scratch::make_writable(dir);
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(anyhow::Error::from(e).context(format!("removing {}", dir.display())))
        }
        _ => Ok(()),
    }
}

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    SafeGit::local(dir)?.args(args).run()
}

/// Removes the worktree at `tree`, registered in `main_clone`: its build
/// output first, then the worktree itself. `recheck` is §15.3 rule 2 again.
/// The bytes freed, or why it stopped.
pub fn worktree(
    tree: &Path,
    main_clone: &Path,
    recheck: &dyn Fn() -> Result<(), String>,
) -> Result<u64, Outcome> {
    let bytes = size_of(tree);
    for name in BUILD_OUTPUT {
        let out = tree.join(name);
        let Ok(meta) = std::fs::symlink_metadata(&out) else {
            continue;
        };
        if super::scope::is_link(&meta) || !meta.is_dir() {
            continue;
        }
        recheck().map_err(Outcome::Held)?;
        retrying(&out, &HELD_WAITS_MS, recheck, || remove_dir(&long(&out)))?;
    }
    recheck().map_err(Outcome::Held)?;
    let shown = tree.display().to_string();
    // `worktree remove` runs `status` in the tree; the emptied filters reach
    // it through git's own command-line config.
    let removed = super::unsaved::without_filters(tree, main_clone)
        .and_then(|g| g.args(&["worktree", "remove", &shown]).run());
    if let Err(error) = removed {
        let text = format!("{error:#}");
        if text.contains("modified or untracked") || text.contains("is locked") {
            return Err(Outcome::Held(format!("git kept {shown}: {text}")));
        }
        if !tree.exists() {
            // Removed after all (a race with another pass): only the
            // registration may be left, and the prune drops it.
        } else if cfg!(windows) {
            // git passed its clean check and stopped on a file it couldn't
            // delete: what is left goes by path, held files retried.
            retrying(tree, &HELD_WAITS_MS, recheck, || remove_dir(&long(tree)))?;
        } else {
            return Err(Outcome::Failed(format!("removing {shown}: {text}")));
        }
    }
    git(main_clone, &["worktree", "prune"])
        .map_err(|e| Outcome::Failed(format!("{shown} was removed but the prune failed: {e:#}")))?;
    Ok(bytes)
}

/// Whether a bot's shared build cache of `cache_bytes` goes (§15.2): when
/// the bot has no other open PR or worktree, the disk has under 20 GB free,
/// or the cache is over its 30 GB cap.
pub fn trims_cache(no_other_work: bool, free_bytes: Option<u64>, cache_bytes: u64) -> bool {
    const LOW_DISK: u64 = 20_000_000_000;
    const CACHE_CAP: u64 = 30_000_000_000;
    no_other_work || free_bytes.is_some_and(|f| f < LOW_DISK) || cache_bytes > CACHE_CAP
}

/// Trims the bot's shared Cargo target beside `workspace` when
/// [`trims_cache`] says so: the bytes freed.
pub fn trim_cache(workspace: &Path, no_other_work: bool) -> u64 {
    if !workspace.is_absolute() {
        return 0;
    }
    let cache = crate::workers::target::bot_target(workspace);
    let Ok(meta) = std::fs::symlink_metadata(&cache) else {
        return 0;
    };
    if super::scope::is_link(&meta) || !meta.is_dir() {
        return 0;
    }
    let bytes = size_of(&cache);
    let free = crate::migrate_home::disk::free_bytes(workspace);
    if !trims_cache(no_other_work, free, bytes) {
        return 0;
    }
    match remove_dir(&long(&cache)) {
        Ok(()) => bytes,
        Err(error) => {
            tracing::warn!(cache = %cache.display(), %error, "trimming a bot's build cache failed");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn held() -> anyhow::Error {
        anyhow::Error::from(std::io::Error::from_raw_os_error(32)).context("removing x")
    }

    #[test]
    fn a_held_file_is_retried_then_held_with_the_links_checked_each_time() {
        let dir = tempfile::tempdir().unwrap();
        let (tries, checks) = (Cell::new(0), Cell::new(0));
        let recheck = || {
            checks.set(checks.get() + 1);
            Ok(())
        };
        let out = retrying(dir.path(), &[1, 1, 1], &recheck, || {
            tries.set(tries.get() + 1);
            Err(held())
        });
        assert_eq!(tries.get(), 4);
        assert_eq!(checks.get(), 3);
        match out {
            Err(Outcome::Held(why)) => assert!(why.contains("still held"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_retry_stops_when_a_link_appears_and_a_brief_hold_passes() {
        let dir = tempfile::tempdir().unwrap();
        let swapped = || Err("now a link".to_string());
        let tries = Cell::new(0);
        let out = retrying(dir.path(), &[1, 1], &swapped, || {
            tries.set(tries.get() + 1);
            Err(held())
        });
        assert_eq!(out, Err(Outcome::Held("now a link".into())));
        assert_eq!(tries.get(), 1, "nothing tried after the link appeared");

        let tries = Cell::new(0);
        let fine = || Ok(());
        let out = retrying(dir.path(), &[1, 1], &fine, || {
            tries.set(tries.get() + 1);
            if tries.get() < 2 {
                Err(held())
            } else {
                Ok(())
            }
        });
        assert_eq!(out, Ok(()));
        let other = retrying(dir.path(), &[1], &fine, || anyhow::bail!("no such thing"));
        assert!(matches!(other, Err(Outcome::Failed(_))));
    }

    #[test]
    fn the_cache_goes_only_by_the_rule() {
        let gb = 1_000_000_000;
        assert!(trims_cache(true, Some(200 * gb), gb));
        assert!(trims_cache(false, Some(19 * gb), gb));
        assert!(trims_cache(false, Some(200 * gb), 31 * gb));
        assert!(!trims_cache(false, Some(200 * gb), 29 * gb));
        assert!(!trims_cache(false, None, gb));
    }
}
