//! Whether a PR may merge (H-261 §5.1): every condition, each missing one
//! named as a blocker so the owner sees what it waits for (UX-051 decision 4).
//! Main moves only to a head that descends from its tip, so the merge is
//! exactly the commit reviewed and checked.

use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::release::git_cache;
use crate::prs::check_model::CheckResult;
use crate::prs::model::Pr;
use crate::prs::owner;
use crate::prs::repo;
use crate::prs::review_model::{Review, Verdict};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    /// `hermes.pr.v1.BlockerKind`, lowercase without its prefix.
    pub kind: &'static str,
    pub text: String,
    pub subject: Option<String>,
    pub paths: Vec<String>,
}

impl Blocker {
    fn new(kind: &'static str, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            subject: None,
            paths: Vec::new(),
        }
    }

    fn about(mut self, subject: &str) -> Self {
        self.subject = Some(subject.to_string());
        self
    }
}

/// What still stands between the PR and main; empty means mergeable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mergeable {
    pub blockers: Vec<Blocker>,
}

impl Mergeable {
    pub fn ok(&self) -> bool {
        self.blockers.is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "ok": self.ok(),
            "blockers": self.blockers.iter().map(|b| json!({
                "kind": b.kind, "text": b.text, "subject": b.subject, "paths": b.paths,
            })).collect::<Vec<_>>(),
        })
    }
}

/// The latest review a role gave, and whether it approves the current change.
fn latest<'a>(reviews: &'a [Review], role: &str) -> Option<&'a Review> {
    reviews.iter().rev().find(|r| r.role == role)
}

/// The §5.1 conditions, read now. `exact` also asks git whether a head
/// behind main would conflict (for display); the queue's tick skips it.
pub fn compute(app: &AppState, pr: &Pr, exact: bool) -> anyhow::Result<Mergeable> {
    let mut out = Vec::new();
    // 1. Open, with its head reported and verified.
    if !pr.state.is_live() {
        out.push(Blocker::new(
            "not_open",
            format!("PR #{} is {}", pr.number, pr.state.as_str()),
        ));
        return Ok(Mergeable { blockers: out });
    }
    if pr.moved_unreported {
        out.push(Blocker::new(
            "moved_unreported",
            "The branch moved without a reported push",
        ));
    }
    let (needs, reviews, shape, checks) = app.db.board_read(|t| {
        Ok((
            t.pr_needs(&pr.id)?,
            t.reviews(&pr.id)?,
            t.pr_shape(&pr.id)?,
            t.checks_on(&pr.project_id, &pr.head_sha)?,
        ))
    })?;
    // 2. Every required role approved this change; the owner too if needed.
    for role in needs.iter().filter(|r| *r != "owner") {
        match latest(&reviews, role) {
            Some(r) if r.verdict == Verdict::ChangesRequested => out.push(
                Blocker::new("changes_requested", format!("{role} asked for changes")).about(role),
            ),
            Some(r) if !r.stale(pr) => {}
            Some(_) => out.push(
                Blocker::new("review_missing", format!("{role} approved an older change"))
                    .about(role),
            ),
            None => {
                out.push(Blocker::new("review_missing", format!("Waiting for {role}")).about(role))
            }
        }
    }
    let settings = app.db.review_settings(&pr.project_id)?;
    let owner_needed = owner::required(&settings, &shape, pr) || needs.iter().any(|r| r == "owner");
    if owner_needed && settings.mode != owner::Mode::None {
        let fresh = latest(&reviews, "owner")
            .is_some_and(|r| r.verdict == Verdict::Approved && !r.stale(pr));
        if !fresh {
            out.push(Blocker::new("owner_review", "Waiting for your review").about("owner"));
        }
    }
    // 3. No unresolved `must` from the latest round.
    let roles: std::collections::BTreeSet<&str> = reviews.iter().map(|r| r.role.as_str()).collect();
    for role in roles {
        let Some(r) = latest(&reviews, role) else {
            continue;
        };
        let open = r
            .findings
            .iter()
            .filter(|f| f.severity == "must" && f.resolved_in.is_none())
            .count();
        if open > 0 && r.verdict == Verdict::Approved && !r.stale(pr) {
            out.push(
                Blocker::new(
                    "unresolved_must",
                    format!("{open} must-fix finding(s) from {role} are open"),
                )
                .about(role),
            );
        }
    }
    // ...and no open `must` comment thread (H-282).
    let musts = crate::prs::comments::open_musts(app, pr)?;
    if musts > 0 {
        out.push(
            Blocker::new(
                "unresolved_must",
                format!("{musts} must-fix comment(s) are open"),
            )
            .about("comments"),
        );
    }
    // 4. Every required check passed on the head (or its tree).
    for c in checks.iter().filter(|c| c.required) {
        match c.result {
            CheckResult::Pass => {}
            CheckResult::Fail | CheckResult::Error => out.push(
                Blocker::new("check_failed", format!("{} didn't pass", c.name)).about(&c.name),
            ),
            CheckResult::Queued | CheckResult::Running => out.push(
                Blocker::new(
                    "check_pending",
                    format!("{} is {}", c.name, c.result.as_str()),
                )
                .about(&c.name),
            ),
        }
    }
    // 5. The head descends from main's tip: main fast-forwards to it.
    let cache = repo::of_project(app, &pr.project_id, Some(&pr.repo))?.cached(app);
    if let Some(main) = git_cache::resolve(&cache, "refs/heads/main") {
        if !git_cache::contains(&cache, &pr.head_sha, &main)? {
            let paths = if exact {
                git_cache::conflicts(&cache, &main, &pr.head_sha).unwrap_or_default()
            } else {
                Vec::new()
            };
            if paths.is_empty() {
                out.push(Blocker::new("behind_main", "Needs update with main"));
            } else {
                let mut b = Blocker::new("conflicts", "Has conflicts with main");
                b.paths = paths;
                out.push(b);
            }
        }
    }
    // 6. The card isn't blocked and its ACs, bar post-install ones, are ticked.
    if let Some(item) = app.db.get_item(&pr.item_id)? {
        if item.blocked.is_some() {
            out.push(Blocker::new(
                "card_blocked",
                format!("{} is blocked", item.id),
            ));
        }
        let open = item
            .acceptance_criteria
            .iter()
            .filter(|a| !a.post_install && !a.checked)
            .count();
        if open > 0 {
            out.push(Blocker::new(
                "ac_unticked",
                format!("{open} acceptance criteria not ticked"),
            ));
        }
    }
    Ok(Mergeable { blockers: out })
}
