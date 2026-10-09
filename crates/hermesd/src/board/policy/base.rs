//! The policy as the PR's base has it (H-261 §2): read at the base commit in
//! the daemon's own copy of the repository (`git_cache`), never from the
//! PR's head or any bot's checkout.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use crate::app::AppState;
use crate::board::release::git_cache;
use crate::decisions::forbidden;

use super::{
    parse_checks, parse_reviewers, required_roles, Checks, ReviewRole, Reviewers, CHECKS_PATH,
    POLICY_ROLES, REVIEWERS_PATH,
};

/// Both policy files at one base commit. A file the base lacks reads as
/// empty; one it has but that doesn't parse keeps its error, to be shown.
#[derive(Debug, Clone)]
pub struct BasePolicy {
    pub commit: String,
    pub reviewers: Result<Reviewers, String>,
    pub checks: Result<Checks, String>,
}

impl BasePolicy {
    /// The roles a change needs under the base's rules. While the base's
    /// `reviewers.toml` is broken, every PR needs what a policy change does,
    /// so the PR that fixes it can still be reviewed and nothing gets less.
    pub fn required_roles(&self, changed: &[String]) -> BTreeSet<ReviewRole> {
        match &self.reviewers {
            Ok(reviewers) => required_roles(reviewers, changed),
            Err(_) => {
                let mut roles = required_roles(&Reviewers::default(), changed);
                roles.extend(POLICY_ROLES);
                roles
            }
        }
    }
}

/// The policy at `base_ref` (a branch, tag or commit) in `cache`.
pub fn read(cache: &Path, base_ref: &str) -> anyhow::Result<BasePolicy> {
    let commit = git_cache::resolve(cache, base_ref)
        .ok_or_else(|| forbidden(format!("the base {base_ref} isn't in the repository")))?;
    let reviewers = match git_cache::file_at(cache, &commit, REVIEWERS_PATH)? {
        None => Ok(Reviewers::default()),
        Some(text) => parse_reviewers(&text).map_err(|e| e.to_string()),
    };
    let checks = match git_cache::file_at(cache, &commit, CHECKS_PATH)? {
        None => Ok(Checks::default()),
        Some(text) => parse_checks(&text).map_err(|e| e.to_string()),
    };
    Ok(BasePolicy {
        commit,
        reviewers,
        checks,
    })
}

/// The project's policy at `base_ref`, after fetching the cache.
pub fn load(app: &Arc<AppState>, project: &str, base_ref: &str) -> anyhow::Result<BasePolicy> {
    let url = app
        .db
        .project_repo(project)?
        .map(|r| r.url)
        .ok_or_else(|| {
            forbidden("the project has no repository set, so its policy can't be read")
        })?;
    let cache = git_cache::refresh(&app.cfg.home, project, &url)?;
    read(&cache, base_ref)
}

#[cfg(test)]
#[path = "base_tests.rs"]
mod tests;
