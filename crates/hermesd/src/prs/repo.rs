//! A PR's repository as the daemon reads it (H-261 §1.2, §3): its own
//! blob-less cache, never a bot's checkout, so a head is what the remote
//! holds and its patch-id is the PR's own change.

use std::path::PathBuf;

use crate::app::AppState;
use crate::board::release::git::github_https;
use crate::board::release::git_cache;

/// A repository a project's PRs live in.
pub struct Repo {
    /// "owner/name" (GitHub), or the URL for any other remote.
    pub name: String,
    pub url: String,
    /// The cache's key: the project's own repository keeps the release
    /// cache; another shares nothing with it.
    key: String,
}

/// "owner/name" for a GitHub URL; any other URL as itself, trimmed.
pub fn name_of(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    match github_https(url) {
        Some(https) => https.trim_start_matches("https://github.com/").to_string(),
        None => url.to_string(),
    }
}

/// Whether two remote URLs name the same repository, however spelled
/// (`git@github.com:o/r.git`, `https://github.com/o/r`).
pub fn same(a: &str, b: &str) -> bool {
    name_of(a).eq_ignore_ascii_case(&name_of(b))
}

/// The repository `asked` names ("owner/name"), or the project's own.
pub fn of_project(app: &AppState, project_id: &str, asked: Option<&str>) -> anyhow::Result<Repo> {
    let own = app.db.project_repo(project_id)?.map(|r| r.url);
    let asked = asked.map(str::trim).filter(|a| !a.is_empty());
    match (own, asked) {
        (Some(url), None) => Ok(Repo {
            name: name_of(&url),
            url,
            key: project_id.to_string(),
        }),
        (Some(url), Some(name)) if same(&url, name) => Ok(Repo {
            name: name_of(&url),
            url,
            key: project_id.to_string(),
        }),
        (_, Some(name)) => {
            let ok = name.split('/').count() == 2
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
                && !name.contains("..");
            anyhow::ensure!(ok, "repo must be a GitHub \"owner/name\", not {name:?}");
            Ok(Repo {
                name: name.to_string(),
                url: format!("https://github.com/{name}.git"),
                key: format!("{project_id}--{}", name.replace('/', "--")),
            })
        }
        (None, None) => anyhow::bail!(
            "the project has no repository set; the owner sets it in the project's settings"
        ),
    }
}

impl Repo {
    /// The daemon's cache of it, fetched now.
    pub fn fetch(&self, app: &AppState) -> anyhow::Result<PathBuf> {
        git_cache::refresh(&app.cfg.home, &self.key, &self.url)
    }

    /// The cache without fetching, if it exists.
    pub fn cached(&self, app: &AppState) -> PathBuf {
        git_cache::dir(&app.cfg.home, &self.key)
    }
}

/// A head's facts in the cache: main's tip, and the patch-id of the change
/// from its merge-base with main to the head (§3).
pub struct Facts {
    pub base_sha: String,
    pub patch_id: String,
}

pub fn facts(cache: &std::path::Path, head: &str) -> anyhow::Result<Facts> {
    let main = git_cache::resolve(cache, "refs/heads/main")
        .ok_or_else(|| anyhow::anyhow!("the repository has no main branch"))?;
    let base = git_cache::merge_base(cache, &main, head)?;
    Ok(Facts {
        base_sha: main,
        patch_id: git_cache::patch_id(cache, &base, head)?,
    })
}

/// The branch's tip on the remote, as fetched into the cache.
pub fn tip(cache: &std::path::Path, branch: &str) -> Option<String> {
    git_cache::resolve(cache, &format!("refs/heads/{branch}"))
}
