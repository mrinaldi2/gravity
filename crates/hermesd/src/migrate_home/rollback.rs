//! `migrate-home --rollback`: undoes a run from its action log.

use std::io::Write;
use std::time::Duration;

use anyhow::{bail, Context};

use super::{daemon_stopped, disk, files, sql, steps, Action, Plan, Step, STATE_FILE};

fn is_real_dir(path: &std::path::Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
}

/// Reverses a migration from its log, newest change first, and leaves the
/// old home exactly where it was. The daemon must be stopped; after the new
/// service has run, uninstall it first (`hermesd service uninstall`).
pub fn rollback(plan: &Plan, out: &mut dyn Write) -> anyhow::Result<()> {
    let Some(mut state) = plan.state()? else {
        bail!("no migration to roll back (no {STATE_FILE})");
    };
    let home = if state.done(Step::Move) {
        &plan.to
    } else {
        &plan.from
    };
    daemon_stopped(home, Duration::from_secs(10)).context("stop the daemon first")?;
    while let Some(action) = state.actions.pop() {
        writeln!(out, "undo {action:?}")?;
        match &action {
            Action::Symlink { path } => steps::remove_link(path)?,
            Action::File { path, original } => files::write(path, original)?,
            Action::Database { .. } => {
                sql::apply(&plan.to.join("bus.sqlite"), &plan.reversed(), false)?;
            }
            Action::Rename { from, to } => {
                // Recreated by a process that kept writing to the old path
                // (the Symlink step refuses to run over it): set aside,
                // never merged into or deleted.
                if from == &plan.from && is_real_dir(from) {
                    let aside = disk::set_aside(from)?;
                    writeln!(
                        out,
                        "{} was recreated after the move; moved it to {}, merge it by hand",
                        from.display(),
                        aside.display()
                    )?;
                }
                if from.symlink_metadata().is_ok() {
                    bail!(
                        "cannot move {} back: {} exists",
                        to.display(),
                        from.display()
                    );
                }
                // A merged transcript dir was emptied and removed.
                if let Some(parent) = from.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(to, from).with_context(|| {
                    format!("moving {} back to {}", to.display(), from.display())
                })?;
                if from == &plan.from {
                    state.steps.retain(|s| *s != Step::Move);
                }
            }
        }
        state.save()?;
    }
    std::fs::remove_file(plan.from.join(STATE_FILE))?;
    writeln!(out, "rolled back; {} is as it was", plan.from.display())?;
    Ok(())
}
