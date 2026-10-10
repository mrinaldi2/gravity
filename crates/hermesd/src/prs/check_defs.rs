//! Which definition each check runs for a PR that changes `checks.toml`
//! (H-311): a check that can never pass must not block its own fix, and a
//! PR that changes a check must say plainly what that does to its checks
//! (CE M1).
//!
//! "What the PR changes" is the head's file against the file at the
//! merge-base, never against today's main: on a branch behind main, main's
//! own later edits aren't the PR's (CE S3). A check the PR leaves alone runs
//! main's current definition; one it changes or adds runs the head's; one it
//! removes doesn't run. A passing `checks.toml` row names each kind:
//! - `<name> runs its own definition`, `<name> added`;
//! - `<name> skipped: no longer required`, `<name> skipped: paths no longer
//!   match`: main's definition would run on this PR, the head's doesn't;
//! - `<name> removed`.
//!
//! Such a PR always needs architect, ce and the owner (a policy change).

use crate::board::policy::{Check, Checks};

use super::check_model::{CheckResult, NewCheck};
use super::checks::{policy_check, queue};

/// The checks of a PR whose head's `checks.toml` parses and differs from
/// the merge-base's: `current` is main's, `before` the merge-base's, `head`
/// the PR's; `changed` the paths the PR changes.
pub(super) fn with_head_defs(
    current: &Checks,
    before: &Checks,
    head: &Checks,
    changed: &[String],
) -> Vec<NewCheck> {
    let find = |set: &'_ Checks, name: &str| set.checks.iter().find(|c| c.name == name).cloned();
    let runs = |c: &Check| c.required && c.applies(changed);
    let mut names: Vec<&str> = Vec::new();
    for c in current
        .checks
        .iter()
        .chain(&before.checks)
        .chain(&head.checks)
    {
        if !names.contains(&c.name.as_str()) {
            names.push(&c.name);
        }
    }
    let mut merged = Checks::default();
    let mut said = Vec::new();
    for name in names {
        let (now, was, theirs) = (find(current, name), find(before, name), find(head, name));
        if theirs == was {
            // The PR leaves it alone: main's definition, if main still has it.
            merged.checks.extend(now);
            continue;
        }
        let Some(theirs) = theirs else {
            said.push(format!("{name} removed"));
            continue;
        };
        let would = now.as_ref().is_some_and(runs);
        if runs(&theirs) {
            said.push(match now {
                Some(_) => format!("{name} runs its own definition"),
                None => format!("{name} added"),
            });
        } else if would {
            said.push(if theirs.required {
                format!("{name} skipped: paths no longer match")
            } else {
                format!("{name} skipped: no longer required")
            });
        }
        merged.checks.push(theirs);
    }
    let mut checks = queue(&merged, changed);
    if !said.is_empty() {
        checks.push(policy_check(
            CheckResult::Pass,
            &format!("this PR's checks.toml: {}", said.join("; ")),
        ));
    }
    checks
}

#[cfg(test)]
#[path = "check_defs_tests.rs"]
mod tests;
