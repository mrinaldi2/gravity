//! `pr_diff` (H-273; H-261 §9): a PR's change as a file list with +/−
//! counts and a unified diff, from the daemon's blob-less cache through
//! SafeGit. Any range of commits the cache holds can be asked, so the app
//! shows head, one push, or `approved_sha..head` after an approval went
//! stale. The diff stops at 2 MB with `truncated`: the rest is for a terminal.

use std::collections::HashMap;
use std::path::Path;

use bus::contract::pr as p;

use crate::app::AppState;
use crate::board::release::git_cache;
use crate::decisions::invalid;
use crate::prs::model::Pr;
use crate::prs::repo;

/// The most diff text one answer carries.
pub const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;

/// The diff the request asks of `pr`: `from` defaults to the merge-base of
/// main and `to`, `to` to the head.
pub fn serve(app: &AppState, pr: &Pr, r: &p::PrDiffRequest) -> anyhow::Result<p::PrDiff> {
    let cache = repo::of_project(app, &pr.project_id, Some(&pr.repo))?.cached(app);
    let to = commit(&cache, r.to_sha.as_deref().unwrap_or(&pr.head_sha))?;
    let from = match r.from_sha.as_deref() {
        Some(sha) => commit(&cache, sha)?,
        None => git_cache::merge_base(&cache, &format!("refs/heads/{}", pr.base), &to)
            .or_else(|_| git_cache::merge_base(&cache, &pr.base_sha, &to))?,
    };
    let path = r.path.as_deref().map(str::trim).filter(|p| !p.is_empty());
    if let Some(path) = path {
        // After `--` git still reads `:(…)` as pathspec magic.
        if path.starts_with(':') || path.contains('\0') {
            return Err(invalid(format!("{path:?} isn't a path in the repository")));
        }
    }
    let files = files(&cache, &from, &to, path)?;
    let (diff, truncated) = capped(&git_cache::diff(&cache, &from, &to, path, &[])?);
    Ok(p::PrDiff {
        from_sha: from,
        to_sha: to,
        files,
        diff,
        truncated,
    })
}

/// A full commit id the cache holds. Only hex reaches git, so no request
/// can pass git an option or a revision expression.
fn commit(cache: &Path, sha: &str) -> anyhow::Result<String> {
    let sha = sha.trim();
    let hex = (4..=64).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit());
    if !hex {
        return Err(invalid(format!("{sha:?} isn't a commit id")));
    }
    git_cache::resolve(cache, sha)
        .filter(|full| git_cache::has(cache, full))
        .ok_or_else(|| invalid(format!("{sha} isn't a commit of this repository")))
}

/// The text up to the last whole line within the cap, and whether it was cut.
pub fn capped(raw: &[u8]) -> (String, bool) {
    let text = String::from_utf8_lossy(raw);
    if text.len() <= MAX_DIFF_BYTES {
        return (text.into_owned(), false);
    }
    let mut end = MAX_DIFF_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let end = text[..end].rfind('\n').map_or(end, |i| i + 1);
    (text[..end].to_string(), true)
}

fn files(
    cache: &Path,
    from: &str,
    to: &str,
    path: Option<&str>,
) -> anyhow::Result<Vec<p::DiffFile>> {
    let counts = numstat(&git_cache::diff(
        cache,
        from,
        to,
        path,
        &["--numstat", "-z"],
    )?);
    let raw = git_cache::diff(cache, from, to, path, &["--name-status", "-z"])?;
    Ok(name_status(&raw)
        .into_iter()
        .map(|mut file| {
            match counts.get(&file.path) {
                Some(Some((added, deleted))) => {
                    file.additions = *added;
                    file.deletions = *deleted;
                }
                Some(None) => file.binary = true,
                None => {}
            }
            file
        })
        .collect())
}

/// `--numstat -z`: per new path, `(added, deleted)`, or `None` for binary.
pub fn numstat(raw: &[u8]) -> HashMap<String, Option<(u32, u32)>> {
    let text = String::from_utf8_lossy(raw);
    let mut fields = text.split('\0');
    let mut out = HashMap::new();
    while let Some(entry) = fields.next() {
        let mut parts = entry.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        // A rename leaves the path empty; the old and new paths follow.
        let path = if path.is_empty() {
            fields.next();
            fields.next().unwrap_or_default()
        } else {
            path
        };
        let counts = a.parse().ok().zip(d.parse().ok());
        out.insert(path.to_string(), counts);
    }
    out
}

/// `--name-status -z`: each file with its status, and its old path when renamed.
pub fn name_status(raw: &[u8]) -> Vec<p::DiffFile> {
    use p::FileStatus as S;
    let text = String::from_utf8_lossy(raw);
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut out = Vec::new();
    while let Some(status) = fields.next() {
        let (status, old_path, path) = match status.chars().next() {
            Some('R') => {
                let old = fields.next().unwrap_or_default();
                (S::Renamed, old, fields.next().unwrap_or_default())
            }
            // A copy is a new file as far as the reader goes.
            Some('C') => {
                fields.next();
                (S::Added, "", fields.next().unwrap_or_default())
            }
            Some('A') => (S::Added, "", fields.next().unwrap_or_default()),
            Some('D') => (S::Deleted, "", fields.next().unwrap_or_default()),
            _ => (S::Modified, "", fields.next().unwrap_or_default()),
        };
        out.push(p::DiffFile {
            path: path.to_string(),
            old_path: old_path.to_string(),
            status: status as i32,
            ..Default::default()
        });
    }
    out
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
