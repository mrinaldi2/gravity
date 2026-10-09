//! Line comments on a PR (H-261 §1.4; H-269 the owner's, H-282 the bots'):
//! a comment names a commit the PR has had, a path and a line, and is shown
//! on a newer head where that line went, or `outdated` when it is gone.
//! Replies join their thread; resolving closes it. An open `must` thread
//! keeps the PR from merging (§5.1(3)).

use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::release::git_cache;
use crate::db::comments::{Comment, NewComment};
use crate::decisions::{forbidden, invalid, not_found};
use crate::prs::model::Pr;
use crate::prs::{anchor, repo};

/// What a comment says, from the owner's WS or a bot's tool.
pub struct Write<'a> {
    pub sha: &'a str,
    pub path: &'a str,
    pub line: u32,
    pub side: &'a str,
    pub body: &'a str,
    pub severity: Option<&'a str>,
    pub reply_to: Option<&'a str>,
}

fn side(s: &str) -> anyhow::Result<&'static str> {
    match s {
        "" | "new" | "SIDE_NEW" => Ok("new"),
        "old" | "SIDE_OLD" => Ok("old"),
        other => Err(invalid(format!("side is old or new, not {other}"))),
    }
}

fn severity(s: Option<&str>) -> anyhow::Result<Option<&'static str>> {
    match s.unwrap_or_default() {
        "" | "SEVERITY_UNSPECIFIED" => Ok(None),
        "must" | "SEVERITY_MUST" => Ok(Some("must")),
        "should" | "SEVERITY_SHOULD" => Ok(Some("should")),
        "nit" | "SEVERITY_NIT" => Ok(Some("nit")),
        other => Err(invalid(format!(
            "severity is must, should or nit, not {other}"
        ))),
    }
}

/// Records a comment by `author` (a bot id, or `owner:<provenance>`). A
/// reply takes its thread's anchor and never a severity of its own.
pub fn add(
    app: &AppState,
    project: &str,
    number: u32,
    w: &Write<'_>,
    author: &str,
) -> anyhow::Result<Comment> {
    let body = w.body.trim();
    if body.is_empty() {
        return Err(invalid("a comment needs a body"));
    }
    let side = side(w.side)?;
    let severity = severity(w.severity)?;
    // The anchor is checked before the transaction: reading the repository
    // takes the database lock the transaction holds.
    let reply = w.reply_to.is_some_and(|r| !r.is_empty());
    if !reply {
        let pr = app
            .db
            .board_read(|t| t.pr(project, number))?
            .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
        let (sha, path) = (w.sha.trim(), w.path.trim());
        let known = pr.head_sha == sha
            || app
                .db
                .board_read(|t| t.pr_pushes(&pr.id))?
                .iter()
                .any(|p| p.sha == sha);
        if !known {
            return Err(invalid(format!("{sha} isn't a commit of PR #{number}")));
        }
        if !path.is_empty() && w.line > 0 {
            check_anchor(app, &pr, sha, path, w.line, side)?;
        }
    }
    app.db.board_tx(|t| {
        let pr = t
            .pr(project, number)?
            .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
        if let Some(parent) = w.reply_to.filter(|r| !r.is_empty()) {
            let parent = t
                .comment(parent)?
                .filter(|c| c.pr_id == pr.id)
                .ok_or_else(|| not_found(format!("no comment {parent} on PR #{number}")))?;
            let root = parent.reply_to.clone().unwrap_or(parent.id.clone());
            return t.insert_comment(&NewComment {
                pr_id: &pr.id,
                sha: &parent.sha,
                path: &parent.path,
                line: parent.line,
                side: &parent.side,
                body,
                author,
                severity: None,
                reply_to: Some(&root),
            });
        }
        let (sha, path) = (w.sha.trim(), w.path.trim());
        if sha.is_empty() || path.is_empty() || w.line == 0 {
            return Err(invalid("a comment needs sha, path and a line from 1"));
        }
        let known = pr.head_sha == sha || t.pr_pushes(&pr.id)?.iter().any(|p| p.sha == sha);
        if !known {
            return Err(invalid(format!("{sha} isn't a commit of PR #{number}")));
        }
        t.insert_comment(&NewComment {
            pr_id: &pr.id,
            sha,
            path,
            line: w.line,
            side,
            body,
            author,
            severity,
            reply_to: None,
        })
    })
}

/// The anchor names a line of a text file at that commit (ARCH M2): on the
/// `new` side the commit itself, on the `old` side its merge-base with the
/// PR's base. A missing path, a folder, a binary file or a line past the
/// end is refused.
fn check_anchor(
    app: &AppState,
    pr: &Pr,
    sha: &str,
    path: &str,
    line: u32,
    side: &str,
) -> anyhow::Result<()> {
    let cache = repo::of_project(app, &pr.project_id, Some(&pr.repo))?.cached(app);
    let at = if side == "old" {
        git_cache::merge_base(&cache, &pr.base_sha, sha)?
    } else {
        sha.to_string()
    };
    let lines = git_cache::line_count(&cache, &at, path)?.ok_or_else(|| {
        invalid(format!(
            "{path} isn't a text file at {}",
            &at[..at.len().min(7)]
        ))
    })?;
    if line as usize > lines {
        return Err(invalid(format!(
            "{path} has {lines} lines there, not {line}"
        )));
    }
    Ok(())
}

/// Resolves a thread (ARCH M1). A `must` thread only by its author or the
/// owner, and one the owner started only by the owner, so a PR's author never
/// clears a reviewer's must-fix; other threads also by the PR's author.
pub fn resolve(
    app: &AppState,
    project: &str,
    number: u32,
    id: &str,
    by: &str,
    by_owner: bool,
) -> anyhow::Result<Comment> {
    app.db.board_tx(|t| {
        let pr = t
            .pr(project, number)?
            .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
        let c = t
            .comment(id)?
            .filter(|c| c.pr_id == pr.id)
            .ok_or_else(|| not_found(format!("no comment {id} on PR #{number}")))?;
        let root = match &c.reply_to {
            Some(root) => t
                .comment(root)?
                .ok_or_else(|| not_found("no such thread"))?,
            None => c,
        };
        let owners = root.author.starts_with("owner:");
        let must = root.severity.as_deref() == Some("must");
        let allowed =
            by_owner || (!owners && root.author == by) || (!owners && !must && pr.author == by);
        if !allowed {
            return Err(forbidden(if owners {
                "only the owner resolves a thread the owner started"
            } else if must {
                "a must-fix thread is resolved by its author or the owner"
            } else {
                "the thread's author, the PR's author or the owner resolves it"
            }));
        }
        t.resolve_comment(&root.id, by)?;
        t.comment(&root.id)?
            .ok_or_else(|| not_found("no such thread"))
    })
}

/// A comment as shown on `target`: `shown_line` where its line is there, or
/// `outdated`.
pub fn shown(app: &AppState, pr: &Pr, c: &Comment, target: &str) -> anyhow::Result<Value> {
    let cache = repo::of_project(app, &pr.project_id, Some(&pr.repo))?.cached(app);
    let line = anchor::anchor(&cache, &c.sha, target, &c.path, c.line, &c.side)?;
    Ok(json!({
        "id": c.id, "sha": c.sha, "path": c.path, "line": c.line, "side": c.side,
        "body": c.body, "author": c.author, "severity": c.severity, "reply_to": c.reply_to,
        "resolved": c.resolved_at.is_some(), "resolved_by": c.resolved_by,
        "shown_on": target, "shown_line": line, "outdated": line.is_none(), "at": c.at,
    }))
}

/// A PR's comments as shown on `target` (its head when `None`).
pub fn list(app: &AppState, pr: &Pr, target: Option<&str>) -> anyhow::Result<Vec<Value>> {
    let target = target.unwrap_or(&pr.head_sha);
    let comments = app.db.board_read(|t| t.comments(&pr.id))?;
    comments.iter().map(|c| shown(app, pr, c, target)).collect()
}

/// Open `must` threads: they keep the PR from merging (§5.1(3)).
pub fn open_musts(app: &AppState, pr: &Pr) -> anyhow::Result<usize> {
    let comments = app.db.board_read(|t| t.comments(&pr.id))?;
    Ok(comments
        .iter()
        .filter(|c| {
            c.reply_to.is_none() && c.severity.as_deref() == Some("must") && c.resolved_at.is_none()
        })
        .count())
}

/// The owner's comment over WS (H-269), with its provenance.
pub fn add_owner(
    app: &AppState,
    project: &str,
    number: u32,
    req: &Value,
    provenance: &str,
) -> anyhow::Result<Value> {
    let text = |k: &str| {
        req.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
    };
    let line = req.get("line").and_then(Value::as_u64).unwrap_or(0);
    let c = add(
        app,
        project,
        number,
        &Write {
            sha: text("sha"),
            path: text("path"),
            line: u32::try_from(line).unwrap_or(0),
            side: text("side"),
            body: text("body"),
            severity: Some(text("severity")),
            reply_to: Some(text("reply_to")),
        },
        &format!("owner:{provenance}"),
    )?;
    let pr = app
        .db
        .board_read(|t| t.pr(project, number))?
        .ok_or_else(|| not_found(format!("no PR #{number}")))?;
    shown(app, &pr, &c, &pr.head_sha)
}
