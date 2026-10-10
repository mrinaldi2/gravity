//! Unsaved work in a worktree, and saving it before anything is kept or
//! removed (H-261 §15.3 rule 4). Everything runs through [`SafeGit`] with
//! plumbing: the tree is a bot's, so its config must not make this git run
//! a program. A diff passes `--no-ext-diff --no-textconv`, and SafeGit
//! empties every filter driver the tree's config names (H-295). That
//! matters even for `diff-index`: a file whose stat matches the index too
//! closely to trust (racy git) is read and hashed, through the clean filter
//! its attributes name.

use std::path::{Component, Path, PathBuf};

use crate::safe_git::SafeGit;
use crate::worktree::METADATA_FILE;

/// What a worktree holds that no remote has.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Unsaved {
    /// Tracked paths that differ from HEAD (a stat-only change counts).
    pub changed: Vec<String>,
    /// Untracked paths that aren't ignored.
    pub untracked: Vec<String>,
    /// Commits on HEAD that neither a remote ref nor the merged commit has.
    pub unpushed: u32,
}

impl Unsaved {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.untracked.is_empty() && self.unpushed == 0
    }

    /// "2 uncommitted path(s), 1 unpushed commit(s)".
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        let uncommitted = self.changed.len() + self.untracked.len();
        if uncommitted > 0 {
            parts.push(format!("{uncommitted} uncommitted path(s)"));
        }
        if self.unpushed > 0 {
            parts.push(format!("{} unpushed commit(s)", self.unpushed));
        }
        parts.join(", ")
    }
}

fn git(tree: &Path, args: &[&str]) -> anyhow::Result<String> {
    SafeGit::local(tree)?.args(args).run()
}

fn lines(out: &str) -> Vec<String> {
    out.lines()
        .filter(|l| !l.is_empty() && *l != METADATA_FILE)
        .map(str::to_string)
        .collect()
}

/// `HEAD --not --remotes [merged]`: what the count and the bundle take.
fn unpushed_range(tree: &Path, merged: &str) -> Vec<String> {
    let mut range = vec!["HEAD".to_string(), "--not".into(), "--remotes".into()];
    let known = !merged.is_empty()
        && git(tree, &["cat-file", "-e", &format!("{merged}^{{commit}}")]).is_ok();
    if known {
        range.push(merged.to_string());
    }
    range
}

/// What `tree` holds that would be lost if it were removed.
pub fn find(tree: &Path, merged: &str) -> anyhow::Result<Unsaved> {
    let changed = SafeGit::local(tree)?
        .args(&[
            "diff-index",
            "--name-only",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
        ])
        .run()?;
    let untracked = git(tree, &["ls-files", "--others", "--exclude-standard"])?;
    let mut count = vec!["rev-list".to_string(), "--count".into()];
    count.extend(unpushed_range(tree, merged));
    let unpushed = SafeGit::local(tree)?.args(&count).run()?;
    Ok(Unsaved {
        changed: lines(&changed),
        untracked: lines(&untracked),
        unpushed: unpushed.trim().parse().unwrap_or(u32::MAX),
    })
}

/// An untracked file bigger than this isn't copied (ARCH S1 on H-274)…
pub const FILE_CAP: u64 = 100_000_000;
/// …nor any once the copy would pass this in all…
pub const TOTAL_CAP: u64 = 1_000_000_000;
/// …or leave the disk with less than this free.
pub const FREE_FLOOR: u64 = 5_000_000_000;

/// Where a salvage went, and the untracked files it left in the tree.
#[derive(Debug)]
pub struct Salvaged {
    pub dir: PathBuf,
    /// "path (size, why)": kept in the tree, which is held, never copied.
    pub skipped: Vec<String>,
}

/// Saves what `unsaved` names into `dir`: a bundle of the unpushed commits,
/// a patch of the tracked changes, and a copy of the untracked files by
/// their bytes (links skipped, never followed) within the caps.
pub fn salvage(
    tree: &Path,
    merged: &str,
    unsaved: &Unsaved,
    dir: &Path,
) -> anyhow::Result<Salvaged> {
    std::fs::create_dir_all(dir)?;
    if unsaved.unpushed > 0 {
        let bundle = dir.join("commits.bundle");
        let mut args = vec![
            "bundle".to_string(),
            "create".into(),
            bundle.display().to_string(),
        ];
        args.extend(unpushed_range(tree, merged));
        SafeGit::local(tree)?.args(&args).run()?;
    }
    if !unsaved.changed.is_empty() {
        let out = SafeGit::local(tree)?
            .args(&[
                "diff",
                "--binary",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "HEAD",
                "--",
            ])
            .output()?;
        anyhow::ensure!(
            out.status.success(),
            "git diff: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        std::fs::write(dir.join("uncommitted.patch"), &out.stdout)?;
    }
    let mut sized = Vec::new();
    for rel in &unsaved.untracked {
        if let Some(bytes) = copyable(tree, rel)? {
            sized.push((rel.clone(), bytes));
        }
    }
    let free = crate::migrate_home::disk::free_bytes(dir);
    let (copy, skipped) = plan_copy(sized, free);
    for rel in &copy {
        let to = dir.join("untracked").join(rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(tree.join(rel), &to)?;
    }
    Ok(Salvaged {
        dir: dir.to_path_buf(),
        skipped,
    })
}

/// The untracked files to copy, in order, and those left out with why:
/// each within [`FILE_CAP`], all within [`TOTAL_CAP`], and the disk kept
/// above [`FREE_FLOOR`] (an unknown free space counts as none to spare).
pub fn plan_copy(sized: Vec<(String, u64)>, free: Option<u64>) -> (Vec<String>, Vec<String>) {
    let room = free.map_or(0, |f| f.saturating_sub(FREE_FLOOR));
    let (mut copy, mut skipped, mut total) = (Vec::new(), Vec::new(), 0u64);
    for (rel, bytes) in sized {
        let size = super::model::human_bytes(bytes);
        let why = if bytes > FILE_CAP {
            Some("over the 100 MB cap")
        } else if total + bytes > TOTAL_CAP {
            Some("past the 1 GB salvage cap")
        } else if total + bytes > room {
            Some("not enough free disk")
        } else {
            None
        };
        match why {
            Some(why) => skipped.push(format!("{rel} ({size}, {why})")),
            None => {
                total += bytes;
                copy.push(rel);
            }
        }
    }
    (copy, skipped)
}

/// An untracked file's size, unless it or a folder above it is a link or it
/// isn't a plain file (then it isn't copied at all).
fn copyable(tree: &Path, rel: &str) -> anyhow::Result<Option<u64>> {
    let rel = Path::new(rel);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Ok(None);
    }
    let mut at = tree.to_path_buf();
    for part in rel.components() {
        at.push(part);
        if super::scope::is_link(&std::fs::symlink_metadata(&at)?) {
            return Ok(None);
        }
    }
    let meta = std::fs::symlink_metadata(&at)?;
    Ok(meta.is_file().then_some(meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_salvage_copy_keeps_to_its_caps() {
        let mb = 1_000_000;
        let plenty = Some(100_000 * mb);
        let sized = |list: &[(&str, u64)]| list.iter().map(|(p, b)| (p.to_string(), *b)).collect();
        let (copy, skipped) = plan_copy(sized(&[("a", mb), ("big", 150 * mb), ("b", mb)]), plenty);
        assert_eq!(copy, ["a", "b"]);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].starts_with("big (") && skipped[0].contains("100 MB cap"));

        let many: Vec<(String, u64)> = (0..12).map(|i| (format!("f{i}"), 90 * mb)).collect();
        let (copy, skipped) = plan_copy(many, plenty);
        assert_eq!(copy.len(), 11, "11 x 90 MB fit in 1 GB");
        assert!(skipped[0].contains("1 GB salvage cap"), "{skipped:?}");

        let (copy, skipped) = plan_copy(sized(&[("a", mb)]), Some(FREE_FLOOR + mb / 2));
        assert!(
            copy.is_empty() && skipped[0].contains("free disk"),
            "{skipped:?}"
        );
        let (copy, _) = plan_copy(sized(&[("a", mb)]), None);
        assert!(copy.is_empty(), "an unknown free space spares nothing");
    }
}
