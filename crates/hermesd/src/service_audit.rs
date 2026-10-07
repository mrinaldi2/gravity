//! Who ran `service install` (H-193). An install replaces the binary and
//! restarts the service and every bot; once, a newer app did it on launch
//! and nothing on disk said which code path had. Each install now appends
//! one line to `<logs>/service-install.log`: when, which version, its
//! arguments, and the process that ran it with what launched that one.

use std::io::Write;
use std::path::Path;

use crate::bus_auth::origin::{exe_path, name_of};
use crate::bus_auth::os::OsProcessTable;
use crate::bus_auth::session::ProcessTable;

/// The log's file name in the service's log directory.
pub const LOG_FILE: &str = "service-install.log";

/// Appends this install's line to `log_dir`. Best effort: a log that can't
/// be written never stops the install.
pub fn record(log_dir: &Path, args: &[String]) {
    let line = line(&chrono::Utc::now().to_rfc3339(), args, &caller());
    let written = std::fs::create_dir_all(log_dir).and_then(|()| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join(LOG_FILE))
            .and_then(|mut file| file.write_all(line.as_bytes()))
    });
    if let Err(error) = written {
        eprintln!(
            "could not record the install in {}: {error}",
            log_dir.display()
        );
    }
}

/// The process that ran this one and the one that launched it, e.g.
/// `/Applications/The Hermes.app/Contents/MacOS/hermes-desktop (pid 36676), launched from launchd`.
fn caller() -> String {
    let table = OsProcessTable;
    let Some(parent) = table.info(std::process::id()).map(|me| me.ppid) else {
        return "an unknown process".to_string();
    };
    let exe = exe_path(parent).map_or_else(|| "?".to_string(), |p| p.display().to_string());
    let launcher = table
        .info(parent)
        .and_then(|info| name_of(info.ppid))
        .unwrap_or_else(|| "?".to_string());
    format!("{exe} (pid {parent}), launched from {launcher}")
}

fn line(at: &str, args: &[String], caller: &str) -> String {
    format!(
        "{at} hermesd {} {} by {caller}\n",
        env!("CARGO_PKG_VERSION"),
        args.join(" ")
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_line_says_when_what_and_who() {
        let args = ["service", "install", "--no-migrate"].map(String::from);
        let line = super::line("2026-10-07T03:22:31Z", &args, "/A/hermes-desktop (pid 7)");
        assert_eq!(
            line,
            format!(
                "2026-10-07T03:22:31Z hermesd {} service install --no-migrate by /A/hermes-desktop (pid 7)\n",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn the_caller_is_this_test_runner_s_parent() {
        let dir = tempfile::tempdir().expect("dir");
        super::record(dir.path(), &["service".to_string(), "install".to_string()]);
        let text = std::fs::read_to_string(dir.path().join(super::LOG_FILE)).expect("log");
        assert!(text.contains(" service install by "), "{text}");
        assert!(text.contains("(pid "), "{text}");
    }
}
