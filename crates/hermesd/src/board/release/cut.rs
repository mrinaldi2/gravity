//! Releases cut from main (H-272; H-261 §6.1–6.3, ruling fc46b042): a
//! release is a commit on main, and what is in it is computed, not listed.
//!
//! `release_cut {version, commit?, release_id?, repo?}` (DevOps):
//! - the commit defaults to main's tip and must be on main (an ancestor of
//!   the tip, in the daemon's fetch);
//! - its range is `(previous tag's commit, commit]`: the PRs merged there are
//!   the release's PRs, their cards its items;
//! - cut from a planned package, the planned cards not merged yet show as
//!   `not_merged`, and merged cards nobody planned as `also_included`;
//! - nothing is built yet: the gate then runs as before (builds from that
//!   commit, tests, submit, the owner's ruling). No release branch.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::db::{BoardTx, NewCut, NewRelease};
use crate::decisions::{conflict, invalid};
use crate::prs::model::{Pr, PrState};
use crate::prs::repo;

use super::git_cache;
use super::model::{Cut, Release, ReleaseEvent, ReleaseStatus};
use super::{load, Caller};

pub struct CutRequest<'a> {
    pub version: &'a str,
    pub commit: Option<&'a str>,
    pub release_id: Option<&'a str>,
    pub repo: Option<&'a str>,
}

/// What a commit on main holds since the last tag.
pub struct Range {
    pub repo: String,
    pub commit: String,
    pub previous: Option<String>,
    /// Every PR merged in the range, oldest first.
    pub prs: Vec<Pr>,
    /// Their cards, but those whose every PR there was left out by revert.
    pub items: Vec<String>,
}

/// The tag a release from main gets.
pub fn tag_name(release: &Release) -> String {
    format!("desktop-v{}", version_of(release))
}

pub fn version_of(release: &Release) -> String {
    release
        .display_version
        .clone()
        .unwrap_or_else(|| release.name.clone())
        .trim_start_matches('v')
        .to_string()
}

fn check_version(version: &str) -> anyhow::Result<&str> {
    let v = version.trim().trim_start_matches('v');
    let ok = !v.is_empty()
        && v.len() <= 40
        && v.starts_with(|c: char| c.is_ascii_digit())
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c));
    if !ok {
        return Err(invalid(format!(
            "{version:?} isn't a version a tag can carry (like 0.18.0)"
        )));
    }
    Ok(v)
}

/// The commit to cut: `asked` (default main's tip), which must be on main.
pub fn on_main(cache: &Path, asked: Option<&str>) -> anyhow::Result<String> {
    let tip = git_cache::resolve(cache, "refs/heads/main")
        .ok_or_else(|| anyhow::anyhow!("the repository has no main branch"))?;
    let Some(asked) = asked.map(str::trim).filter(|a| !a.is_empty()) else {
        return Ok(tip);
    };
    let commit = git_cache::resolve(cache, asked)
        .ok_or_else(|| invalid(format!("{asked} isn't a commit in the repository")))?;
    if !git_cache::contains(cache, &tip, &commit)? {
        return Err(invalid(format!(
            "{commit} isn't on main (main is at {tip}): a release is cut from main only"
        )));
    }
    Ok(commit)
}

/// The range a release cut at `commit` covers, read now.
pub fn range(
    app: &AppState,
    cache: &Path,
    project: &str,
    repo: &str,
    commit: &str,
    release_id: Option<&str>,
) -> anyhow::Result<Range> {
    let (tagged, merged, reverts) = app.db.board_read(|t| {
        Ok((
            t.tagged_cuts(project, repo)?,
            t.prs(project, &[PrState::Merged])?,
            t.reverted_prs()?,
        ))
    })?;
    // The latest tagged release before this commit.
    let mut previous: Option<String> = None;
    for (id, at) in tagged {
        if Some(id.as_str()) == release_id || at == commit {
            continue;
        }
        if !git_cache::contains(cache, commit, &at)? {
            continue;
        }
        let later = match &previous {
            Some(best) => git_cache::contains(cache, &at, best)?,
            None => true,
        };
        if later {
            previous = Some(at);
        }
    }
    let mut prs = Vec::new();
    for pr in merged.into_iter().filter(|p| p.repo == repo) {
        let Some(sha) = pr.merged_sha.as_deref() else {
            continue;
        };
        let in_range = git_cache::contains(cache, commit, sha)?
            && match &previous {
                Some(p) => !git_cache::contains(cache, p, sha)?,
                None => true,
            };
        if in_range {
            prs.push(pr);
        }
    }
    prs.sort_by_key(|p| (p.merged_at, p.number));
    let left_out: BTreeSet<&str> = reverts
        .iter()
        .flat_map(|(pr, revert)| [pr.as_str(), revert.as_str()])
        .collect();
    let mut items: Vec<String> = Vec::new();
    for pr in &prs {
        let kept = prs
            .iter()
            .filter(|p| p.item_id == pr.item_id)
            .any(|p| !left_out.contains(p.id.as_str()));
        if kept && !items.contains(&pr.item_id) {
            items.push(pr.item_id.clone());
        }
    }
    Ok(Range {
        repo: repo.to_string(),
        commit: commit.to_string(),
        previous,
        prs,
        items,
    })
}

/// `release_cut`: a new release, or a planned one, at a commit on main.
pub fn cut(app: &Arc<AppState>, me: &Caller<'_>, req: &CutRequest<'_>) -> anyhow::Result<Release> {
    me.require(Role::Devops, "cut a release")?;
    let version = check_version(req.version)?;
    let project = me.bot.project_id.as_str();
    let repo = repo::of_project(app, project, req.repo)?;
    let cache = repo.fetch(app)?;
    let commit = on_main(&cache, req.commit)?;
    let range = range(app, &cache, project, &repo.name, &commit, req.release_id)?;
    if range.items.is_empty() {
        return Err(conflict(format!(
            "no pull request merged on main since the last tagged release ({}); nothing to cut",
            range.previous.as_deref().unwrap_or("none")
        )));
    }
    let by = format!("bot:{}", me.bot.id);
    app.db.board_tx(|t| {
        let release = match req.release_id {
            Some(id) => {
                let release = load(t, project, id)?;
                if release.status != ReleaseStatus::Planned || !release.builds.is_empty() {
                    return Err(conflict(format!(
                        "release {} is {}: only a planned package is cut",
                        release.name,
                        release.status.as_str()
                    )));
                }
                release
            }
            None => {
                if t.releases(project)?.iter().any(|r| r.name == version) {
                    return Err(conflict(format!(
                        "this project already has a release named {version}"
                    )));
                }
                t.insert_release(&NewRelease {
                    project_id: project,
                    name: version,
                    display_version: Some(version),
                    changelog: "",
                    how_to_test: &json!([]),
                    created_by: &me.bot.id,
                    items: &[],
                })?
            }
        };
        let planned: Vec<String> = if release.status == ReleaseStatus::Planned {
            release.items.iter().map(|i| i.item_id.clone()).collect()
        } else {
            Vec::new()
        };
        write(t, &release, &range, &planned, &by, "cut")?;
        if release.status == ReleaseStatus::Planned {
            t.assemble_plan(&release.id)?;
        }
        Ok(t.release(&release.id)?.expect("just cut"))
    })
}

/// Records a (re-)cut on `release`: its range, its items, an event.
pub fn write(
    t: &BoardTx<'_>,
    release: &Release,
    range: &Range,
    planned: &[String],
    by: &str,
    kind: &str,
) -> anyhow::Result<()> {
    for id in &range.items {
        if let Some(other) = t
            .open_releases_of_item(id)?
            .into_iter()
            .find(|other| *other != release.id)
        {
            return Err(conflict(format!(
                "{id} is already in release {other}; cancel that package first if it is \
                 still being assembled"
            )));
        }
    }
    let now: Vec<String> = release.items.iter().map(|i| i.item_id.clone()).collect();
    let add: Vec<String> = range
        .items
        .iter()
        .filter(|i| !now.contains(i))
        .cloned()
        .collect();
    let remove: Vec<String> = now
        .iter()
        .filter(|i| !range.items.contains(i))
        .cloned()
        .collect();
    t.change_release_items(&release.id, &add, &remove)?;
    let pr_ids: Vec<String> = range.prs.iter().map(|p| p.id.clone()).collect();
    t.set_release_cut(
        &release.id,
        &NewCut {
            repo: &range.repo,
            source_commit: &range.commit,
            previous_commit: range.previous.as_deref(),
            planned,
        },
        &pr_ids,
    )?;
    let not_merged: Vec<&String> = planned
        .iter()
        .filter(|p| !range.items.contains(p))
        .collect();
    let event = ReleaseEvent {
        release_id: release.id.clone(),
        release_name: release.name.clone(),
        related_id: None,
        kind: kind.to_string(),
        actor: by.to_string(),
        note: Some(format!(
            "cut from main at {} ({} pull requests)",
            short(&range.commit),
            range.prs.len()
        )),
        detail: json!({
            "commit": range.commit, "previous": range.previous,
            "prs": range.prs.iter().map(|p| p.number).collect::<Vec<_>>(),
            "items": range.items, "not_merged": not_merged,
        }),
        at: Utc::now(),
    };
    t.record_release_event(&event, &release.project_id)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn pr_ref(cut: &Cut, p: &super::model::CutPr) -> Value {
    json!({ "number": p.number, "item_id": p.item_id, "merged_sha": p.merged_sha,
            "title": p.title, "repo": cut.repo, "reverted": p.reverted, "revert": p.revert })
}

/// The `hermes.pr.v1.ReleaseFromMain` keys on the release JSON, plus the
/// cards planned but not merged and where the range starts.
pub fn extend_json(release: &Release, cut: &Cut, out: &mut Value) {
    let planned = |item: &str| cut.planned.iter().any(|p| p == item);
    let (prs, also): (Vec<_>, Vec<_>) = cut
        .prs
        .iter()
        .partition(|p| cut.planned.is_empty() || planned(&p.item_id));
    let items: Vec<&str> = release.items.iter().map(|i| i.item_id.as_str()).collect();
    let not_merged: Vec<&String> = cut
        .planned
        .iter()
        .filter(|p| !items.contains(&p.as_str()))
        .collect();
    let extra = json!({
        "tag": cut.tag.clone().unwrap_or_default(),
        "tag_name": tag_name(release),
        "commit": cut.source_commit,
        "previous_commit": cut.previous_commit,
        "repo": cut.repo,
        "prs": prs.iter().map(|p| pr_ref(cut, p)).collect::<Vec<_>>(),
        "also_included": also.iter().map(|p| pr_ref(cut, p)).collect::<Vec<_>>(),
        "not_merged": not_merged,
    });
    if let (Some(out), Some(extra)) = (out.as_object_mut(), extra.as_object()) {
        out.extend(extra.clone());
    }
}
