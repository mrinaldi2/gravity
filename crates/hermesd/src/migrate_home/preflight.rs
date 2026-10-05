//! What a run would do, and what would stop it: `migrate-home --dry-run`,
//! and the checks every run makes before it touches anything.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{disk, files, sql, steps, Plan, State, Step};
use crate::holders::Owned;

/// Space kept free beyond the backup itself.
const DISK_MARGIN: u64 = 512 * 1024 * 1024;

/// Everything a run would touch, and anything that would stop it.
pub fn dry_run(plan: &Plan, out: &mut dyn Write) -> anyhow::Result<bool> {
    Ok(report(plan, None, out)?.is_empty())
}

/// What would stop a run, apart from the daemon it is about to stop and
/// the processes in `owned` (that daemon's own, which the install stops
/// with it): `service install` checks this before it stops anything, so a
/// process holding the home blocks the install while everything still runs.
/// The run checks again once the daemon has stopped.
pub fn blockers_before_stop(plan: &Plan, owned: &Owned) -> anyhow::Result<Vec<String>> {
    report(plan, Some(owned), &mut std::io::sink())
}

const DAEMON_RUNNING: &str = "a daemon is running against";

/// `before_stop` holds the daemon's processes when the daemon is still to
/// be stopped (see [`blockers_before_stop`]).
fn report(
    plan: &Plan,
    before_stop: Option<&Owned>,
    out: &mut dyn Write,
) -> anyhow::Result<Vec<String>> {
    let state = plan.state()?;
    let mut blockers = checks(plan, state.as_ref(), before_stop);
    writeln!(
        out,
        "migrate {} -> {}",
        plan.from.display(),
        plan.to.display()
    )?;
    if let Some(state) = &state {
        writeln!(out, "resuming; done so far: {:?}", state.steps)?;
    }
    let home = if state.as_ref().is_some_and(|s| s.done(Step::Move)) {
        &plan.to
    } else {
        &plan.from
    };
    for (old, new) in steps::renamed_files() {
        if home.join(&old).exists() {
            writeln!(out, "rename    {} -> {}", old.display(), new.display())?;
        }
    }
    let db = home.join("bus.sqlite");
    if db.is_file() {
        for (column, rows) in sql::count(&db, &plan.path_pairs())? {
            writeln!(out, "rewrite   {column}: {rows} row(s)")?;
        }
    }
    for file in files::pending(plan, home) {
        writeln!(out, "rewrite   {}", file.display())?;
    }
    let transcripts = steps::transcript_dirs(plan)?;
    let mut needed = disk::tree_size(&db) + DISK_MARGIN;
    for (old, new) in &transcripts {
        needed += disk::tree_size(old);
        writeln!(out, "rename    {} -> {}", old.display(), new.display())?;
    }
    writeln!(
        out,
        "link      {} -> {}",
        plan.from.display(),
        plan.to.display()
    )?;
    match disk::free_bytes(home) {
        Some(free) => {
            writeln!(
                out,
                "disk      {} MB free, {} MB needed",
                free >> 20,
                needed >> 20
            )?;
            if free < needed && !state.as_ref().is_some_and(|s| s.done(Step::Backup)) {
                blockers.push(format!(
                    "not enough free disk for the backup ({} MB)",
                    needed >> 20
                ));
            }
        }
        None => writeln!(
            out,
            "disk      free space unknown; backup needs {} MB",
            needed >> 20
        )?,
    }
    // One line for whoever confirms the run (the desktop app shows it).
    let bots = if db.is_file() {
        sql::live_bots(&db)?
    } else {
        0
    };
    writeln!(
        out,
        "summary   Moves {} to {}, restarts {bots} bot(s), takes about 30 s, needs {} MB.",
        plan.from.display(),
        plan.to.display(),
        needed >> 20
    )?;
    for blocker in &blockers {
        writeln!(out, "blocked   {blocker}")?;
    }
    Ok(blockers)
}

/// Reasons a run cannot start or resume.
pub(super) fn preflight(plan: &Plan, state: Option<&State>) -> Vec<String> {
    checks(plan, state, None)
}

fn checks(plan: &Plan, state: Option<&State>, before_stop: Option<&Owned>) -> Vec<String> {
    let mut blockers = Vec::new();
    if state.is_some_and(State::is_complete) {
        blockers.push("already migrated".to_string());
        return blockers;
    }
    let moved = state.is_some_and(|s| s.done(Step::Move));
    if !moved {
        if !plan.source_is_real_home() {
            blockers.push(format!("{} holds no daemon home", plan.from.display()));
        }
        if let Ok(mut entries) = std::fs::read_dir(&plan.to) {
            if entries.next().is_some() {
                blockers.push(format!("{} exists and is not empty", plan.to.display()));
            }
        }
    }
    let home = if moved { &plan.to } else { &plan.from };
    let running = home.exists() && daemon_stopped(home, Duration::ZERO).is_err();
    if running && before_stop.is_none() {
        blockers.push(format!("{DAEMON_RUNNING} {}", home.display()));
    } else if !moved && plan.source_is_real_home() {
        blockers.extend(holder_blockers(
            plan,
            before_stop.unwrap_or(&Owned::default()),
        ));
    }
    blockers
}

/// Processes other than the daemon working in or holding files under the old
/// home. On macOS a rename succeeds under them, and one that writes by
/// absolute path afterwards recreates a real, near-empty old home; on Windows
/// they fail the move. Either way they are named, never killed. Those in
/// `owned` are left out.
fn holder_blockers(plan: &Plan, owned: &Owned) -> Vec<String> {
    const SHOWN: usize = 20;
    let roots: Vec<PathBuf> = plan
        .path_pairs()
        .into_iter()
        .map(|(from, _)| PathBuf::from(from))
        .collect();
    let holders = match crate::holders::list(&roots) {
        Ok(mut holders) => {
            holders.retain(|holder| !owned.covers(holder));
            holders
        }
        Err(error) => {
            tracing::warn!(%error, "could not list processes holding the home");
            return Vec::new();
        }
    };
    let mut blockers: Vec<String> = holders.iter().take(SHOWN).map(|h| h.to_string()).collect();
    if holders.len() > SHOWN {
        blockers.push(format!("and {} more", holders.len() - SHOWN));
    }
    if !blockers.is_empty() {
        blockers.push(format!(
            "stop the processes above (they hold {}), then retry",
            plan.from.display()
        ));
    }
    blockers
}

/// Waits up to `wait` for both the old and the new daemon to let go of
/// `home`. The locks are released again before anything moves.
pub(super) fn daemon_stopped(home: &Path, wait: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + wait;
    loop {
        // `lock` claims both the old and the new lock file.
        let held = crate::home::lock(home).map(drop);
        match held {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}
