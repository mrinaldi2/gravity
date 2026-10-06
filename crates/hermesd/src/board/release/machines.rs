//! The computers a release is tested on (H-115): per computer, not per
//! platform. A tester tests on the computer its role names, else the linked
//! computer its bot runs on, else this one. Every one of them must pass a
//! package before it is submitted, unless the owner or lead set the list.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::db::BoardTx;
use crate::decisions::{forbidden, invalid};

/// This computer, for a tester here whose role names no machine. Peers name
/// each other, but a daemon has no name for itself.
pub const THIS_COMPUTER: &str = "this computer";

/// The longest list the owner or lead may set.
const MAX: usize = 20;

/// Each tester and the computer it tests on.
pub fn testers(t: &BoardTx<'_>, project_id: &str) -> anyhow::Result<Vec<(String, String)>> {
    Ok(t.tester_machines(project_id)?
        .into_iter()
        .map(|(bot, machine)| (bot, machine.unwrap_or_else(|| THIS_COMPUTER.into())))
        .collect())
}

/// The computers a package must pass on: the set list, else every tester's.
pub fn required(t: &BoardTx<'_>, project_id: &str) -> anyhow::Result<Vec<String>> {
    let set = t.release_machines(project_id)?;
    if !set.is_empty() {
        return Ok(set);
    }
    let all: BTreeSet<String> = testers(t, project_id)?
        .into_iter()
        .map(|(_, machine)| machine)
        .collect();
    Ok(all.into_iter().collect())
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

/// Every tester's computer for the bot ids it holds there.
pub fn view(t: &BoardTx<'_>, project_id: &str) -> anyhow::Result<Value> {
    let testers = testers(t, project_id)?;
    Ok(json!({
        "project_id": project_id,
        "required": required(t, project_id)?,
        "set": t.release_machines(project_id)?,
        "testers": testers
            .iter()
            .map(|(bot, machine)| json!({ "bot_id": bot, "machine": machine }))
            .collect::<Vec<_>>(),
    }))
}

/// Sets the list: each a computer some tester tests on, so it can report.
/// An empty list goes back to every tester's computer.
pub fn set(t: &BoardTx<'_>, project_id: &str, machines: &[String]) -> anyhow::Result<Value> {
    let known: BTreeSet<String> = testers(t, project_id)?
        .into_iter()
        .map(|(_, machine)| machine)
        .collect();
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
    t.set_release_machines(project_id, &list.into_iter().collect::<Vec<_>>())?;
    view(t, project_id)
}
