//! Ending the managed daemon's process trees.
use std::path::PathBuf;

use anyhow::Context;

use super::{process_tree, same_path};

/// Ends the managed daemon and every process it started. `taskkill /T` finds
/// children through WMI, so a broken WMI left the old daemon running; walk the
/// tree with a Toolhelp snapshot instead.
pub(super) fn stop_daemon(pid: u32, executables: &[PathBuf]) -> anyhow::Result<()> {
    // An absent or reused PID needs no termination.
    let Some(root) = process_tree::Process::open(pid)? else {
        return Ok(());
    };
    let path = root.image_path()?;
    if !executables
        .iter()
        .any(|executable| same_path(executable, &path))
    {
        return Ok(());
    }
    process_tree::kill_tree(root).context("could not stop managed daemon")
}

/// Ends every process tree running one of `executables` and confirms none is
/// left, so a binary can be moved out from under it.
pub(super) fn reap(executables: &[PathBuf]) -> anyhow::Result<()> {
    for process in process_tree::running(executables)? {
        process_tree::kill_tree(process).context("could not stop managed daemon")?;
    }
    let left = process_tree::running(executables)?;
    anyhow::ensure!(
        left.is_empty(),
        "{} managed daemon process(es) still running",
        left.len()
    );
    Ok(())
}
