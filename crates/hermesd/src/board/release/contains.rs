//! Whether a later package contains an older one (H-121, CE-015, CE-018,
//! H-146): the old package's commit is the later one's or in its history,
//! read in the daemon's own copy of the project's repository. Used to close
//! a package through a later one (`deployed_via`) and before a deploy
//! supersedes one (`supersede`, H-191).

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::app::AppState;
use crate::decisions::forbidden;

use super::gates::recorded_commit;
use super::git_cache;
use super::model::Release;

/// How the via package was shown to contain the old one: the old commit is
/// in the via commit's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Basis {
    /// `ancestry` (the commit it recorded) or `release_branch`.
    pub rule: &'static str,
    /// The branch or tag the old commit was read from, for `release_branch`.
    pub reference: Option<String>,
    /// The branch or tag the via commit was read from, when it recorded none.
    pub via_reference: Option<String>,
    /// For `via_reference`: its commit's time and the via package's submit,
    /// the first no later than the second (CE-018 M2).
    pub via_commit_at: Option<DateTime<Utc>>,
    pub via_submitted_at: Option<DateTime<Utc>>,
    pub old: String,
    pub via: String,
}

/// When the via package was submitted: its freeze, else its last build.
fn submitted_at(via: &Release) -> Option<DateTime<Utc>> {
    via.frozen_at
        .or_else(|| via.builds.iter().map(|b| b.built_at).max())
}

/// Whether, and how, `via` contains `old`: its commit in the via commit's
/// history, read in the daemon's own copy of the repository.
pub(super) fn contained(
    app: &Arc<AppState>,
    project: &str,
    old: &Release,
    via: &Release,
) -> anyhow::Result<Basis> {
    let via_recorded = recorded_commit(via)?;
    let recorded = recorded_commit(old)?;
    let url = app
        .db
        .project_repo(project)?
        .map(|r| r.url)
        .ok_or_else(|| forbidden("the project has no repository set, so history can't be read"))?;
    let cache = git_cache::refresh(&app.cfg.home, project, &url)?;
    let (via_reference, via_commit, via_commit_at, via_submitted_at) = match via_recorded {
        Some(commit) => (None, commit, None, None),
        // Its builds were attached without a commit (H-146): its own release
        // branch or tag, as for the old package.
        None => {
            let (reference, commit) = release_ref(&cache, via).ok_or_else(|| {
                forbidden(format!(
                    "can't show {} contains {}: {} records no source commit and has no \
                     release branch or tag; record its source commit, or ask the owner",
                    via.name, old.name, via.name
                ))
            })?;
            // The ref may have moved since: a commit newer than the
            // package's submit isn't what it shipped (CE-018 M2).
            let at = git_cache::commit_time(&cache, &commit);
            let submitted = submitted_at(via);
            match (at, submitted) {
                (Some(at), Some(submitted)) if at <= submitted => {}
                _ => {
                    let name = reference
                        .trim_start_matches("refs/heads/")
                        .trim_start_matches("refs/tags/");
                    return Err(forbidden(format!(
                        "{name} has moved since {} was submitted; record its source commit, \
                         or ask the owner",
                        via.name
                    )));
                }
            }
            (Some(reference), commit, at, submitted)
        }
    };
    let (rule, reference, commit) = match recorded {
        Some(commit) => ("ancestry", None, commit),
        // Recorded before commits were (CE-015 M1): its release branch or
        // tag, never a guess from what it built.
        None => {
            let (reference, commit) = release_ref(&cache, old).ok_or_else(|| {
                forbidden(format!(
                    "can't show {} is contained in {}: it records no source commit and has no \
                     release branch or tag; record its source commit, or ask the owner",
                    old.name, via.name
                ))
            })?;
            ("release_branch", Some(reference), commit)
        }
    };
    for c in [&commit, &via_commit] {
        if !git_cache::has(&cache, c) {
            return Err(forbidden(format!(
                "commit {c} isn't in the project's repository"
            )));
        }
    }
    if git_cache::contains(&cache, &via_commit, &commit)? {
        Ok(Basis {
            rule,
            reference,
            via_reference,
            via_commit_at,
            via_submitted_at,
            old: commit,
            via: via_commit,
        })
    } else {
        Err(forbidden(format!(
            "release {} ({}) doesn't contain {}'s commit {}{}",
            via.name,
            &via_commit[..via_commit.len().min(12)],
            old.name,
            &commit[..commit.len().min(12)],
            reference.map_or_else(String::new, |r| format!(" ({r})"))
        )))
    }
}

/// A package's release branch or tag and its commit:
/// `release/desktop-<v>`, then `desktop-v<v>`, for its display version and
/// then its name.
fn release_ref(cache: &std::path::Path, release: &Release) -> Option<(String, String)> {
    let versions = release
        .display_version
        .iter()
        .chain(std::iter::once(&release.name))
        .map(|v| v.trim().trim_start_matches('v').to_string())
        .filter(|v| !v.is_empty());
    for version in versions {
        for reference in [
            format!("refs/heads/release/desktop-{version}"),
            format!("refs/tags/desktop-v{version}"),
        ] {
            if let Some(commit) = git_cache::resolve(cache, &reference) {
                return Some((reference, commit));
            }
        }
    }
    None
}
