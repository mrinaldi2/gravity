//! When the pause can't clear what holds the home (H-117 R4): the daemon
//! files one owner action on the release's decision, from a fixed template,
//! that stops exactly those processes, each only while it is still the
//! process that was found (same start time), and runs the stop line of any
//! service that failed to stop. Nothing a bot wrote goes into the script:
//! the PIDs and start times are the daemon's own readings, and the service
//! lines the owner's config. Process names and projects go in the reason,
//! which is shown, never run.

use std::process::Command;

use serde_json::Value;

use crate::app::AppState;
use crate::db::Quiesce;
use crate::owner_action::model::{OwnerAction, Proposal, Shell, State};
use crate::owner_action::{self, writable_roots};

use super::services::ServiceOutcome;

/// A process to stop, as found: its pid and start time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub pid: u32,
    pub start: String,
}

/// Only what a start time reading can hold, so it quotes safely.
fn clean_stamp(raw: &str) -> Option<String> {
    let words: Vec<&str> = raw.split_whitespace().collect();
    let stamp = words.join(" ");
    (!stamp.is_empty()
        && stamp
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " :.-".contains(c)))
    .then_some(stamp)
}

/// A process's start time as the script will read it at run time.
pub fn start_stamp(pid: u32) -> Option<String> {
    let out = if cfg!(windows) {
        let script = format!("(Get-Process -Id {pid}).StartTime.Ticks");
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
    } else {
        Command::new("/bin/ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .output()
    }
    .ok()?;
    out.status
        .success()
        .then(|| clean_stamp(&String::from_utf8_lossy(&out.stdout)))
        .flatten()
}

/// The script: each process stopped only while its start time matches,
/// then the failed services' stop lines. Exit 1 if anything failed.
pub fn template(windows: bool, found: &[Found], service_stops: &[String]) -> String {
    let mut lines = Vec::new();
    if windows {
        lines.push("$rc = 0".to_string());
        for f in found {
            lines.push(format!(
                "$x = Get-Process -Id {p} -ErrorAction SilentlyContinue; \
                 if ($x -and [string]$x.StartTime.Ticks -eq '{s}') {{ \
                 Stop-Process -Id {p} -Force; if ($?) {{ 'stopped {p}' }} else {{ $rc = 1 }} }} \
                 else {{ '{p} is gone or was replaced; left alone' }}",
                p = f.pid,
                s = f.start
            ));
        }
        for stop in service_stops {
            lines.push(format!("{stop}; if (-not $?) {{ $rc = 1 }}"));
        }
        lines.push("exit $rc".to_string());
    } else {
        lines.push("rc=0".to_string());
        for f in found {
            lines.push(format!(
                "t=$(/bin/ps -o lstart= -p {p} 2>/dev/null); \
                 if [ \"$(echo $t)\" = '{s}' ]; then kill {p} && echo 'stopped {p}' || rc=1; \
                 else echo '{p} is gone or was replaced; left alone'; fi",
                p = f.pid,
                s = f.start
            ));
        }
        for stop in service_stops {
            lines.push(format!("{stop} || rc=1"));
        }
        lines.push("exit $rc".to_string());
    }
    lines.join("\n")
}

/// Printable, one line, at most 80 characters: what a reason may show.
fn shown(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() && owner_action::validate::hidden(*c).is_none())
        .take(80)
        .collect()
}

/// The action already filed for this pause, while it still waits or runs.
fn open_card(app: &AppState, q: &Quiesce) -> Option<OwnerAction> {
    let id = q.report["owner_action"]["id"].as_str()?;
    let a = app.db.get_owner_action(id).ok()??;
    matches!(a.state, State::Proposed | State::Running).then_some(a)
}

/// Files the owner's Run card for what still holds the home, or returns the
/// one this pause filed already. `None` when there is nothing it can stop.
pub fn file(
    app: &AppState,
    q: &Quiesce,
    unresolved: &[Value],
    services: &[ServiceOutcome],
) -> anyhow::Result<Option<OwnerAction>> {
    if let Some(open) = open_card(app, q) {
        return Ok(Some(open));
    }
    let mut found = Vec::new();
    let mut named = Vec::new();
    for h in unresolved {
        let Some(pid) = h["pid"].as_u64().and_then(|p| u32::try_from(p).ok()) else {
            continue;
        };
        let Some(start) = start_stamp(pid) else {
            continue;
        };
        found.push(Found { pid, start });
        let project = h["project_id"]
            .as_str()
            .and_then(|p| app.db.get_project(p).ok().flatten())
            .map(|p| format!(", project {}", shown(&p.name)))
            .unwrap_or_default();
        let command = shown(h["command"].as_str().unwrap_or("?"));
        named.push(format!("pid {pid} ({command}{project})"));
    }
    let stops: Vec<String> = services
        .iter()
        .filter_map(|s| s.failed_stop.clone())
        .collect();
    if found.is_empty() && stops.is_empty() {
        return Ok(None);
    }
    let release = q
        .release_id
        .as_deref()
        .and_then(|id| app.db.board_read(|t| t.release(id)).ok().flatten());
    let project_id = match (&release, &q.exempt_bot) {
        (Some(r), _) => r.project_id.clone(),
        (None, Some(bot)) => match app.db.get_bot(bot)? {
            Some(b) => b.project_id,
            None => return Ok(None),
        },
        (None, None) => return Ok(None),
    };
    let what = release
        .as_ref()
        .map_or_else(|| q.reason.clone(), |r| r.name.clone());
    let mut reason = format!(
        "The install of {} waits on what the pause couldn't stop:",
        shown(&what)
    );
    if !named.is_empty() {
        reason.push_str(&format!(" {}.", named.join("; ")));
    }
    if !stops.is_empty() {
        reason.push_str(&format!(
            " Services that didn't stop: {}.",
            stops.join("; ")
        ));
    }
    reason.push_str(" Run this, then the install retries.");
    let windows = cfg!(windows);
    let proposal = Proposal {
        project_id,
        proposed_by: "daemon".to_string(),
        item_id: None,
        decision_id: release.and_then(|r| r.decision_id),
        target_machine: app.db.daemon_id()?,
        shell: if windows {
            Shell::Powershell
        } else {
            Shell::Bash
        },
        cwd: app.cfg.user_home.display().to_string(),
        content: template(windows, &found, &stops),
        pinned_files: Vec::new(),
        reason,
        timeout_s: 120,
    };
    owner_action::store(app, proposal, "daemon", writable_roots(app)).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_stops_only_the_processes_found_while_they_are_the_same() {
        let found = [
            Found {
                pid: 41,
                start: "Mon Oct  5 12:00:00 2026"
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            },
            Found {
                pid: 42,
                start: "Tue Oct 6 01:02:03 2026".into(),
            },
        ];
        let script = template(false, &found, &["brew services stop colima".into()]);
        let lines: Vec<&str> = script.lines().collect();
        assert_eq!(lines.len(), 5, "{script}");
        assert!(lines[1].contains("-p 41") && lines[1].contains("= 'Mon Oct 5 12:00:00 2026'"));
        assert!(lines[1].contains("then kill 41 "), "{script}");
        assert!(lines[2].contains("kill 42 ") && !lines[2].contains("41"));
        assert_eq!(lines[3], "brew services stop colima || rc=1");
        // Every kill is behind its start-time check.
        assert_eq!(script.matches("kill ").count(), 2);
        assert_eq!(script.matches("lstart=").count(), 2);
        let ps = template(true, &found[..1], &[]);
        assert!(
            ps.contains("StartTime.Ticks -eq 'Mon Oct 5 12:00:00 2026'"),
            "{ps}"
        );
        assert!(ps.contains("Stop-Process -Id 41"));
    }

    #[test]
    fn only_a_start_time_reading_goes_into_the_script() {
        assert_eq!(
            clean_stamp("  Mon Oct  5 12:00:00 2026\n").as_deref(),
            Some("Mon Oct 5 12:00:00 2026")
        );
        assert_eq!(
            clean_stamp("638640000000000000").as_deref(),
            Some("638640000000000000")
        );
        assert_eq!(clean_stamp("x'; rm -rf ~; echo '"), None);
        assert_eq!(clean_stamp(""), None);
    }

    #[test]
    fn names_shown_in_the_reason_are_one_printable_line() {
        let name = shown("node\nrm -rf ~\u{202e}evil");
        assert!(!name.contains('\n') && !name.contains('\u{202e}'), "{name}");
        assert!(shown(&"x".repeat(200)).len() <= 80);
    }

    #[cfg(unix)]
    #[test]
    fn a_live_process_has_a_start_time_the_script_reads_the_same() {
        let me = std::process::id();
        let stamp = start_stamp(me).expect("stamp");
        let script = template(
            false,
            &[Found {
                pid: 1,
                start: stamp.clone(),
            }],
            &[],
        );
        // The run-time reading of the same process matches what was found.
        let check = format!("t=$(/bin/ps -o lstart= -p {me}); [ \"$(echo $t)\" = '{stamp}' ]");
        let ok = Command::new("/bin/bash")
            .args(["--noprofile", "--norc", "-c", &check])
            .status()
            .unwrap();
        assert!(ok.success(), "{script}");
    }
}
