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

/// Saves what `unsaved` names into `dir`: a bundle of the unpushed commits,
/// a patch of the tracked changes, and a copy of the untracked files by
/// their bytes (links skipped, never followed). Returns `dir`.
pub fn salvage(
    tree: &Path,
    merged: &str,
    unsaved: &Unsaved,
    dir: &Path,
) -> anyhow::Result<PathBuf> {
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
    for rel in &unsaved.untracked {
        copy_untracked(tree, rel, &dir.join("untracked"))?;
    }
    Ok(dir.to_path_buf())
}

/// One untracked file copied by its bytes, unless it or a folder above it
/// is a link.
fn copy_untracked(tree: &Path, rel: &str, into: &Path) -> anyhow::Result<()> {
    let rel = Path::new(rel);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Ok(());
    }
    let mut at = tree.to_path_buf();
    for part in rel.components() {
        at.push(part);
        if super::scope::is_link(&std::fs::symlink_metadata(&at)?) {
            return Ok(());
        }
    }
    if !std::fs::symlink_metadata(&at)?.is_file() {
        return Ok(());
    }
    let to = into.join(rel);
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&at, &to)?;
    Ok(())
}
