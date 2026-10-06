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

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::db::BoardTx;
use crate::decisions::{forbidden, invalid};

use super::model::{Release, ReleaseTargets};

/// The name when even the host has none.
const FALLBACK_NAME: &str = "this computer";
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

fn every_tester(t: &BoardTx<'_>, project_id: &str) -> anyhow::Result<Vec<String>> {
    let all: BTreeSet<String> = testers(t, project_id)?
        .into_iter()
        .map(|(_, machine)| machine)
        .collect();
    Ok(all.into_iter().collect())
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
        Ok(targets_now(t, &release.project_id)?.tested_on)
    } else {
        Ok(release.targets.tested_on.clone())
    }
}

/// The computers `release` must reach before it counts as deployed.
pub fn deploys_to(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<Vec<String>> {
    if release.targets.deploys_to.is_empty() {
        Ok(targets_now(t, &release.project_id)?.deploys_to)
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
