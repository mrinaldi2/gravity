//! Rule 1 (H-261 §15.3): main holds the PR's merged commit, in a fresh fetch
//! of the daemon's own cache. The board's home checks it before it runs or
//! asks for any job; a linked computer checks it again itself before it
//! runs the jobs it was asked to (ARCH M1 on H-274), so a stale or wrong
//! home can't get a live PR's tree removed.

use super::model::Outcome;
use crate::app::AppState;
use crate::board::release::git_cache;
use crate::prs::model::{Pr, PrState};

/// The PR merged and main holds its merged commit. The repository's URL,
/// or the outcome every job gets: busy (retried) when the fetch failed,
/// held when it isn't merged.
pub fn verify_merge(app: &AppState, pr: &Pr) -> Result<String, Outcome> {
    let sha = match (&pr.state, &pr.merged_sha) {
        (PrState::Merged, Some(sha)) => sha.clone(),
        _ => {
            return Err(Outcome::Held(format!(
                "PR #{} isn't merged ({})",
                pr.number,
                pr.state.as_str()
            )))
        }
    };
    main_holds(app, &pr.project_id, &pr.repo, pr.number, &sha)
}

/// Whether main of `repo` (one of `project_id`'s repositories here) holds
/// `sha`, fetched now through SafeGit. The repository's URL when it does.
pub fn main_holds(
    app: &AppState,
    project_id: &str,
    repo: &str,
    number: u32,
    sha: &str,
) -> Result<String, Outcome> {
    let is_hex = |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit());
    if !matches!(sha.len(), 40 | 64) || !is_hex(sha) {
        return Err(Outcome::Held(format!(
            "PR #{number}'s merged commit {sha:?} isn't a commit id"
        )));
    }
    let repo = crate::prs::repo::of_project(app, project_id, Some(repo))
        .map_err(|e| Outcome::Held(format!("{e:#}")))?;
    let cache = repo
        .fetch(app)
        .map_err(|e| Outcome::Busy(format!("couldn't fetch {} to check main: {e:#}", repo.name)))?;
    let main = git_cache::resolve(&cache, "refs/heads/main").unwrap_or_default();
    match git_cache::contains(&cache, &main, sha) {
        Ok(true) => Ok(repo.url),
        _ => Err(Outcome::Held(format!(
            "main doesn't hold PR #{number}'s merged commit {sha}"
        ))),
    }
}
