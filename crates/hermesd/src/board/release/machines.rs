//! The computers a release is tested on and deployed to (H-115, ARCH-R55).
//!
//! Per computer, not per platform. A linked tester tests on its peer; a
//! tester here, on the machine its role names, else this computer, which
//! has a name of its own (`machine_name`, the host's name by default).
//!
//! Every tester's computer must pass a package before it is submitted,
//! unless the owner or lead set the list. Deploys go to every tester's
//! computer unless the owner narrowed it: a lead's list narrows testing
//! only, so no release counts as deployed while a computer runs the old
//! version. Both sets are frozen into the package at submit.
//!
//! An iOS package (every build `ios`) has targets of its own instead
//! (H-176): it is tested on `ios`, by the tester whose role names that
//! target (iOS QA), and deployed to `iphone`, the owner's phone, through the
//! same tester. Those names never count as desktop computers, so desktop
//! packages keep needing every desktop tester's computer.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::db::BoardTx;
use crate::decisions::{forbidden, invalid};

use super::model::{DeployAction, DeployResult, Release, ReleaseTargets};

/// The name when even the host has none.
const FALLBACK_NAME: &str = "this computer";
/// An iOS package's test target: iOS QA's simulator and device run (H-176).
pub const IOS_TEST: &str = "ios";
/// An iOS package's deploy target: the owner's iPhone.
pub const IOS_DEVICE: &str = "iphone";

/// A target of iOS packages, never a desktop computer.
pub fn is_ios_target(machine: &str) -> bool {
    machine.eq_ignore_ascii_case(IOS_TEST) || machine.eq_ignore_ascii_case(IOS_DEVICE)
}

/// A computer can't take the iOS target's name, or `same_target` would
/// alias it with the iPhone (H-237 S1): for `machine_name` and peer names.
pub fn refuse_ios_name(name: &str) -> anyhow::Result<()> {
    if is_ios_target(name.trim()) {
        return Err(invalid(format!(
            "'{}' is the iOS target's name; a computer needs another",
            name.trim()
        )));
    }
    Ok(())
}

/// Whether two target names are the same target: the same computer, or both
/// the iOS target. `ios` and `iphone` are one name (H-231): the lead sets
/// tester@ios, and packages frozen by the H-176 repair deploy to `iphone`.
pub fn same_target(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || (is_ios_target(a) && is_ios_target(b))
}

/// Installed on target `m` and not rolled back since (H-191 S2): a
/// Deploy=Ok that a later Rollback undid doesn't count. `ios` and `iphone`
/// are one target (H-231).
pub fn installed_on(release: &Release, m: &str) -> bool {
    let rows = |action| {
        release
            .deployments
            .iter()
            .filter(move |d| same_target(&d.machine, m) && d.action == action)
    };
    rows(DeployAction::Deploy)
        .filter(|d| d.result == Some(DeployResult::Ok))
        .any(|d| {
            !rows(DeployAction::Rollback)
                .any(|b| b.result == Some(DeployResult::RolledBack) && b.at >= d.at)
        })
}

/// A package whose every build is for iOS.
pub fn is_ios_package(release: &Release) -> bool {
    !release.builds.is_empty()
        && release
            .builds
            .iter()
            .all(|b| b.platform.eq_ignore_ascii_case(IOS_TEST))
}

/// The targets an iOS package freezes: tested on `ios`, deployed to `iphone`.
fn ios_targets() -> ReleaseTargets {
    ReleaseTargets {
        tested_on: vec![IOS_TEST.to_string()],
        tested_set_by: None,
        deploys_to: vec![IOS_DEVICE.to_string()],
        deploys_set_by: None,
    }
}
/// The longest list the owner or lead may set, and the longest name.
const MAX: usize = 20;
const MAX_NAME: usize = 40;

/// Who sets the list: only the owner's narrows deploys too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetBy {
    Owner,
    Lead,
}

impl SetBy {
    fn as_str(self) -> &'static str {
        match self {
            SetBy::Owner => "owner",
            SetBy::Lead => "lead",
        }
    }
}

/// The host's own name: the Mac's local host name (`hostname` there can be
/// a DHCP address), else the computer or host name.
fn host_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let scutil = || {
            let out = std::process::Command::new("scutil")
                .args(["--get", "LocalHostName"])
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        };
        let found = if cfg!(target_os = "macos") {
            scutil()
        } else {
            std::env::var("COMPUTERNAME")
                .ok()
                .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        };
        found
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| FALLBACK_NAME.to_string())
    })
}

/// This computer's name for its testers: as set, else the host's.
pub fn this_computer(t: &BoardTx<'_>) -> anyhow::Result<String> {
    Ok(t.daemon_name()?.unwrap_or_else(|| host_name().to_string()))
}

/// Each tester and the computer it tests on.
pub fn testers(t: &BoardTx<'_>, project_id: &str) -> anyhow::Result<Vec<(String, String)>> {
    let here = this_computer(t)?;
    Ok(t.tester_machines(project_id)?
        .into_iter()
        .map(|(bot, machine)| (bot, machine.unwrap_or_else(|| here.clone())))
        .collect())
}

/// Every desktop tester's computer: an iOS target isn't one.
fn every_tester(t: &BoardTx<'_>, project_id: &str) -> anyhow::Result<Vec<String>> {
    let all: BTreeSet<String> = testers(t, project_id)?
        .into_iter()
        .map(|(_, machine)| machine)
        .filter(|machine| !is_ios_target(machine))
        .collect();
    Ok(all.into_iter().collect())
}

/// The sets `release` would freeze if submitted now: an iOS package's own,
/// else the project's desktop ones.
pub fn targets_for(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<ReleaseTargets> {
    if is_ios_package(release) {
        return Ok(ios_targets());
    }
    targets_now(t, &release.project_id)
}

/// The sets a package submitted now would freeze: tested on the set list,
/// else every tester's computer; deployed to the owner's list, else every
/// tester's computer.
pub fn targets_now(t: &BoardTx<'_>, project_id: &str) -> anyhow::Result<ReleaseTargets> {
    let all = every_tester(t, project_id)?;
    let (set, by) = t.release_machines(project_id)?;
    let tested = if set.is_empty() {
        all.clone()
    } else {
        set.clone()
    };
    let owners = by.as_deref() == Some("owner");
    Ok(ReleaseTargets {
        tested_on: tested,
        tested_set_by: by.clone(),
        deploys_to: if owners { set } else { all },
        deploys_set_by: by.filter(|_| owners),
    })
}

/// The computers `release` must pass on: frozen at submit, or as now.
pub fn tested_on(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<Vec<String>> {
    if release.targets.tested_on.is_empty() {
        Ok(targets_for(t, release)?.tested_on)
    } else {
        Ok(release.targets.tested_on.clone())
    }
}

/// The computers `release` must reach before it counts as deployed.
pub fn deploys_to(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<Vec<String>> {
    if release.targets.deploys_to.is_empty() {
        Ok(targets_for(t, release)?.deploys_to)
    } else {
        Ok(release.targets.deploys_to.clone())
    }
}

/// The computer `bot_id` reports for: `asked` when it is one it tests on,
/// or its only one when nothing is asked.
pub fn reporting_for(
    t: &BoardTx<'_>,
    project_id: &str,
    bot_id: &str,
    asked: &str,
) -> anyhow::Result<String> {
    let mine: Vec<String> = testers(t, project_id)?
        .into_iter()
        .filter(|(bot, _)| bot == bot_id)
        .map(|(_, machine)| machine)
        .collect();
    let asked = asked.trim();
    match (mine.as_slice(), asked) {
        ([], _) => Err(forbidden(
            "only a tester reports a release's result; you have no tester role here",
        )),
        ([one], "") => Ok(one.clone()),
        (_, "") => Err(invalid(format!(
            "you test on {}; say which with 'machine'",
            mine.join(", ")
        ))),
        (_, asked) if mine.iter().any(|m| m.eq_ignore_ascii_case(asked)) => Ok(mine
            .into_iter()
            .find(|m| m.eq_ignore_ascii_case(asked))
            .expect("found")),
        _ => Err(forbidden(format!(
            "you test on {}, not {asked}; each computer's own tester reports for it",
            mine.join(", ")
        ))),
    }
}

/// Who tests where, what a package submitted now needs and reaches, and
/// this computer's name.
pub fn view(t: &BoardTx<'_>, project_id: &str) -> anyhow::Result<Value> {
    let (set, by) = t.release_machines(project_id)?;
    let now = targets_now(t, project_id)?;
    Ok(json!({
        "project_id": project_id,
        "machine_name": this_computer(t)?,
        "required": now.tested_on,
        "deploys_to": now.deploys_to,
        "set": set,
        "set_by": by,
        "testers": testers(t, project_id)?
            .iter()
            .map(|(bot, machine)| json!({ "bot_id": bot, "machine": machine }))
            .collect::<Vec<_>>(),
    }))
}

/// Names this computer for its testers (S1): not a linked peer's name.
pub fn set_name(t: &BoardTx<'_>, name: &str) -> anyhow::Result<()> {
    let name = name.trim();
    let fine = !name.is_empty()
        && name.chars().count() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || " -_.".contains(c));
    if !fine {
        return Err(invalid(format!(
            "'machine_name' is 1–{MAX_NAME} letters, digits, spaces, '-', '_' or '.'"
        )));
    }
    refuse_ios_name(name)?;
    if t.peer_names()?.iter().any(|p| p.eq_ignore_ascii_case(name)) {
        return Err(invalid(format!(
            "{name} is a linked computer's name; this one needs its own"
        )));
    }
    t.set_daemon_name(name)
}

/// Sets the list: each a computer some tester tests on, so it can report.
/// An empty list goes back to every tester's computer. A package already
/// submitted keeps the sets it froze.
pub fn set(
    t: &BoardTx<'_>,
    project_id: &str,
    machines: &[String],
    by: SetBy,
) -> anyhow::Result<Value> {
    let known: BTreeSet<String> = every_tester(t, project_id)?.into_iter().collect();
    let mut list = BTreeSet::new();
    for machine in machines.iter().map(|m| m.trim()) {
        if machine.is_empty() {
            return Err(invalid("a machine name is empty"));
        }
        let Some(name) = known.iter().find(|k| k.eq_ignore_ascii_case(machine)) else {
            let names: Vec<&str> = known.iter().map(String::as_str).collect();
            return Err(invalid(format!(
                "no tester tests on {machine}, so it could never report; testers are on {}",
                if names.is_empty() {
                    "no computer yet".to_string()
                } else {
                    names.join(", ")
                }
            )));
        };
        list.insert(name.clone());
    }
    if list.len() > MAX {
        return Err(invalid(format!("at most {MAX} computers")));
    }
    let list: Vec<String> = list.into_iter().collect();
    t.set_release_machines(project_id, &list, by.as_str())?;
    view(t, project_id)
}
