//! The confirmation a migrating install needs: what `service install` would
//! do to the home, from the bundled daemon's own dry run.

use super::{migration_pending, run_sidecar, user_home};

/// What installing the service would do to the home, when it would migrate
/// it: the bundled daemon's `migrate-home --dry-run` summary and anything
/// that would stop it. `None` when the install migrates nothing.
#[tauri::command]
pub async fn home_migration_summary() -> Result<Option<String>, String> {
    if !migration_pending(&user_home()?) {
        return Ok(None);
    }
    let out = run_sidecar(&["migrate-home", "--dry-run"])?;
    Ok(Some(summarize_dry_run(&String::from_utf8_lossy(
        &out.stdout,
    ))))
}

/// The dry run's `summary` line, then its blockers. The running daemon is
/// always one of them here; the install stops it first, so it is left out.
fn summarize_dry_run(output: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut blocked: Vec<&str> = Vec::new();
    for line in output.lines() {
        if let Some(summary) = line.strip_prefix("summary") {
            parts.push(summary.trim().to_string());
        } else if let Some(blocker) = line.strip_prefix("blocked") {
            let blocker = blocker.trim();
            if !blocker.starts_with("a daemon is running") {
                blocked.push(blocker);
            }
        }
    }
    if parts.is_empty() {
        parts.push("Moves your Hermes data to its new home and restarts every bot.".to_string());
    }
    if !blocked.is_empty() {
        parts.push(format!("It will stop because: {}.", blocked.join("; ")));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dry_run_summary_leaves_out_the_daemon_the_install_stops() {
        let output = "migrate /u/.gravity -> /u/.thehermes\n\
                      summary   Moves /u/.gravity to /u/.thehermes, restarts 3 bot(s), takes about 30 s, needs 600 MB.\n\
                      blocked   a daemon is running against /u/.gravity\n";
        assert_eq!(
            summarize_dry_run(output),
            "Moves /u/.gravity to /u/.thehermes, restarts 3 bot(s), takes about 30 s, needs 600 MB."
        );
        let blocked = format!("{output}blocked   pid 7 python3 (cwd /u/.gravity/serve)\n");
        assert!(summarize_dry_run(&blocked)
            .ends_with("It will stop because: pid 7 python3 (cwd /u/.gravity/serve)."));
        assert!(summarize_dry_run("").starts_with("Moves your Hermes data"));
    }
}
