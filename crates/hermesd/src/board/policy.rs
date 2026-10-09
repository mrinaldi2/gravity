//! Repo-owned review and check policy (H-267, H-261 §2): `.hermes/reviewers.toml`
//! names the reviewer roles each area of the tree needs, `.hermes/checks.toml`
//! the checks a change runs. Both are read from the PR's base, never its
//! head (`base`), so a PR can't change its own rules.
//!
//! A change to the policy files themselves always needs architect, ce and the
//! owner (§12 risk 4). That area is hard-coded here, so no file can remove it.

pub mod base;
pub mod glob;

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::decisions::{forbidden, invalid};

pub const REVIEWERS_PATH: &str = ".hermes/reviewers.toml";
pub const CHECKS_PATH: &str = ".hermes/checks.toml";

/// The hard-coded area: the policy files.
const POLICY_PATHS: &[&str] = &[".hermes/*.toml"];
const POLICY_ROLES: &[ReviewRole] = &[ReviewRole::Architect, ReviewRole::Ce, ReviewRole::Owner];

/// A reviewer role as `reviewers.toml` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewRole {
    Architect,
    Ux,
    Ce,
    Devops,
    Qa,
    Owner,
}

impl ReviewRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewRole::Architect => "architect",
            ReviewRole::Ux => "ux",
            ReviewRole::Ce => "ce",
            ReviewRole::Devops => "devops",
            ReviewRole::Qa => "qa",
            ReviewRole::Owner => "owner",
        }
    }

    pub fn parse(text: &str) -> Option<ReviewRole> {
        [
            ReviewRole::Architect,
            ReviewRole::Ux,
            ReviewRole::Ce,
            ReviewRole::Devops,
            ReviewRole::Qa,
            ReviewRole::Owner,
        ]
        .into_iter()
        .find(|r| r.as_str() == text)
    }

    /// The board role a bot holds to fill it (§2.1); the owner is no bot.
    pub fn board_role(self) -> Option<&'static str> {
        match self {
            ReviewRole::Architect => Some("reviewer.arch"),
            ReviewRole::Ux => Some("reviewer.ux"),
            ReviewRole::Ce => Some("reviewer.ce"),
            ReviewRole::Devops => Some("devops"),
            ReviewRole::Qa => Some("tester"),
            ReviewRole::Owner => None,
        }
    }

    /// The lead may waive any role on a PR but these.
    pub fn waivable(self) -> bool {
        !matches!(self, ReviewRole::Ce | ReviewRole::Owner)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Area {
    pub name: String,
    pub paths: Vec<String>,
    pub roles: Vec<ReviewRole>,
}

/// `.hermes/reviewers.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reviewers {
    #[serde(default, rename = "area")]
    pub areas: Vec<Area>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub name: String,
    pub run: String,
    #[serde(default)]
    pub needs: Vec<String>,
    /// Only this machine runs it (`windows`), else any with the tools.
    #[serde(default)]
    pub machine: Option<String>,
    /// Blocks merge until it passes. Unsaid means required.
    #[serde(default = "yes")]
    pub required: bool,
    /// The changes it runs for; none means every change.
    #[serde(default)]
    pub paths: Vec<String>,
}

fn yes() -> bool {
    true
}

impl Check {
    /// Whether a change to `changed` runs this check.
    pub fn applies(&self, changed: &[String]) -> bool {
        self.paths.is_empty() || any_match(&self.paths, changed)
    }
}

/// `.hermes/checks.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checks {
    #[serde(default, rename = "check")]
    pub checks: Vec<Check>,
}

impl Checks {
    /// The checks a change to `changed` runs.
    pub fn for_change<'a>(&'a self, changed: &'a [String]) -> impl Iterator<Item = &'a Check> {
        self.checks.iter().filter(move |c| c.applies(changed))
    }
}

pub fn parse_reviewers(text: &str) -> anyhow::Result<Reviewers> {
    let reviewers: Reviewers =
        toml::from_str(text).map_err(|e| invalid(format!("{REVIEWERS_PATH}: {e}")))?;
    for area in &reviewers.areas {
        anyhow::ensure!(
            !area.name.trim().is_empty() && !area.paths.is_empty() && !area.roles.is_empty(),
            invalid(format!(
                "{REVIEWERS_PATH}: area {:?} needs a name, paths and roles",
                area.name
            ))
        );
    }
    unique(
        reviewers.areas.iter().map(|a| a.name.as_str()),
        REVIEWERS_PATH,
        "area",
    )?;
    Ok(reviewers)
}

pub fn parse_checks(text: &str) -> anyhow::Result<Checks> {
    let checks: Checks =
        toml::from_str(text).map_err(|e| invalid(format!("{CHECKS_PATH}: {e}")))?;
    for check in &checks.checks {
        anyhow::ensure!(
            !check.name.trim().is_empty() && !check.run.trim().is_empty(),
            invalid(format!(
                "{CHECKS_PATH}: check {:?} needs a name and a run command",
                check.name
            ))
        );
    }
    unique(
        checks.checks.iter().map(|c| c.name.as_str()),
        CHECKS_PATH,
        "check",
    )?;
    Ok(checks)
}

fn unique<'a>(names: impl Iterator<Item = &'a str>, file: &str, what: &str) -> anyhow::Result<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        anyhow::ensure!(
            seen.insert(name),
            invalid(format!("{file}: two {what}s are named {name:?}"))
        );
    }
    Ok(())
}

fn any_match<S: AsRef<str>>(patterns: &[S], changed: &[String]) -> bool {
    changed
        .iter()
        .any(|path| patterns.iter().any(|p| glob::matches(p.as_ref(), path)))
}

/// The areas a change to `changed` falls in.
pub fn matched_areas<'a>(reviewers: &'a Reviewers, changed: &[String]) -> Vec<&'a Area> {
    reviewers
        .areas
        .iter()
        .filter(|area| any_match(&area.paths, changed))
        .collect()
}

/// Whether the change touches the policy files.
pub fn touches_policy(changed: &[String]) -> bool {
    any_match(POLICY_PATHS, changed)
}

/// The roles a change to `changed` needs: the union over the areas its paths
/// match, `architect` when none does, and always architect, ce and the owner
/// when it touches the policy files.
pub fn required_roles(reviewers: &Reviewers, changed: &[String]) -> BTreeSet<ReviewRole> {
    let mut roles: BTreeSet<ReviewRole> = reviewers
        .areas
        .iter()
        .filter(|area| any_match(&area.paths, changed))
        .flat_map(|area| area.roles.iter().copied())
        .collect();
    if roles.is_empty() {
        roles.insert(ReviewRole::Architect);
    }
    if touches_policy(changed) {
        roles.extend(POLICY_ROLES);
    }
    roles
}

/// The lead's waiver of one role on one PR (§2.1): recorded and shown,
/// never silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiver {
    pub role: ReviewRole,
    pub reason: String,
    pub by: String,
}

/// A waiver the lead asks for, if it may be given: only by the lead, with a
/// reason, for a role the PR needs, and never for ce or the owner.
pub fn waive(
    required: &BTreeSet<ReviewRole>,
    role: ReviewRole,
    reason: &str,
    by: &str,
    by_lead: bool,
) -> anyhow::Result<Waiver> {
    if !by_lead {
        return Err(forbidden("only the lead can waive a reviewer role"));
    }
    if !role.waivable() {
        return Err(forbidden(format!(
            "the {} review can never be waived",
            role.as_str()
        )));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(invalid("a waiver needs a reason"));
    }
    if !required.contains(&role) {
        return Err(invalid(format!(
            "this PR doesn't need a {} review, so there is nothing to waive",
            role.as_str()
        )));
    }
    Ok(Waiver {
        role,
        reason: reason.to_string(),
        by: by.to_string(),
    })
}

/// The roles still to review once the waivers are taken off. A waiver for ce
/// or the owner, however it was stored, takes nothing off.
pub fn after_waivers(required: &BTreeSet<ReviewRole>, waivers: &[Waiver]) -> BTreeSet<ReviewRole> {
    required
        .iter()
        .copied()
        .filter(|role| !waivers.iter().any(|w| w.role == *role && role.waivable()))
        .collect()
}

#[cfg(test)]
mod tests;
