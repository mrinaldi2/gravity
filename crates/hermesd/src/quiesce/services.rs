//! Services that hold the home, stopped through their own service manager
//! during a pause and started again after it (H-117 Q3): colima and lima by
//! default, from `[[quiesce.service]]` in `hermesd.toml`. Bots can't edit
//! that file, and the list is read once at daemon start: a list changed
//! since is never run, and the report says so (ARCH-R49).
//!
//! A service is stopped only while it runs **and** one of its processes
//! holds the home (or it is marked `always`). Its VM processes are never
//! signalled; only its own stop command runs, by absolute path, with a
//! timeout. Only what a pause stopped is started again.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::db::Quiesce;
use crate::holders::Holder;

/// How long a service command may run.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// One `[[quiesce.service]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    /// Whether it runs: its output must contain `running_match`.
    pub detect: String,
    pub running_match: String,
    /// Process names that, holding the home, mean the service holds it.
    #[serde(default)]
    pub holder_match: Vec<String>,
    pub stop: String,
    pub start: String,
    /// Stopped whenever it runs, holding the home or not.
    #[serde(default)]
    pub always: bool,
}

/// `[quiesce]` in `hermesd.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuiesceConfig {
    /// Minutes a pause may stay open before it resumes by itself.
    pub deadline_minutes: i64,
    /// Where a service command's program is looked up, in order. Never
    /// `PATH`: a bot could put its own `brew` first there.
    pub search_path: Vec<PathBuf>,
    #[serde(rename = "service")]
    pub services: Vec<ServiceSpec>,
}

impl Default for QuiesceConfig {
    /// Colima and Lima are Mac-only, so Windows ships no services and no
    /// search path; the owner adds their own in `[quiesce]`.
    #[cfg(windows)]
    fn default() -> Self {
        Self {
            deadline_minutes: super::DEFAULT_DEADLINE_MINUTES,
            search_path: Vec::new(),
            services: Vec::new(),
        }
    }

    #[cfg(not(windows))]
    fn default() -> Self {
        let spec =
            |name: &str, detect: &str, running: &str, holders: &[&str], stop: &str, start: &str| {
                ServiceSpec {
                    name: name.into(),
                    detect: detect.into(),
                    running_match: running.into(),
                    holder_match: holders.iter().map(|h| h.to_string()).collect(),
                    stop: stop.into(),
                    start: start.into(),
                    always: false,
                }
            };
        Self {
            deadline_minutes: super::DEFAULT_DEADLINE_MINUTES,
            search_path: ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
            services: vec![
                spec(
                    "colima",
                    "brew services info colima --json",
                    "\"running\": true",
                    &[
                        "limactl",
                        "colima",
                        "com.apple.Virtualization.VirtualMachine",
                        "qemu-system",
                    ],
                    "brew services stop colima",
                    "brew services start colima",
                ),
                spec(
                    "lima",
                    "limactl list --format {{.Name}}:{{.Status}}",
                    "default:Running",
                    &[
                        "limactl",
                        "com.apple.Virtualization.VirtualMachine",
                        "qemu-system",
                    ],
                    "limactl stop default",
                    "limactl start default",
                ),
            ],
        }
    }
}

/// What running a command came to.
#[derive(Debug)]
pub struct Ran {
    pub ok: bool,
    pub output: String,
}

/// `line` with its program found in `search_path`, never through `PATH`.
pub fn resolve(line: &str, search_path: &[PathBuf]) -> Option<(PathBuf, Vec<String>)> {
    let mut words = line.split_whitespace().map(str::to_string);
    let program = words.next()?;
    let found = if Path::new(&program).is_absolute() {
        Some(PathBuf::from(&program)).filter(|p| p.is_file())
    } else {
        search_path
            .iter()
            .map(|dir| dir.join(&program))
            .find(|p| p.is_file())
    }?;
    Some((found, words.collect()))
}

/// Runs `line` as the daemon's user, killed after the timeout.
pub fn run(line: &str, search_path: &[PathBuf]) -> Ran {
    let Some((program, args)) = resolve(line, search_path) else {
        return Ran {
            ok: false,
            output: format!("{line}: program not found in {search_path:?}"),
        };
    };
    let child = Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) => {
            return Ran {
                ok: false,
                output: format!("{line}: {e}"),
            }
        }
    };
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Ran {
                    ok: false,
                    output: format!("{line}: timed out after {}s", COMMAND_TIMEOUT.as_secs()),
                };
            }
        }
    }
    match child.wait_with_output() {
        Ok(out) => Ran {
            ok: out.status.success(),
            output: format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        },
        Err(e) => Ran {
            ok: false,
            output: format!("{line}: {e}"),
        },
    }
}

/// Whether to stop `spec`: it runs, and it holds the home or is `always`.
pub fn should_stop(spec: &ServiceSpec, running: bool, holders: &[Holder]) -> bool {
    if !running {
        return false;
    }
    spec.always
        || holders.iter().any(|h| {
            let name = Path::new(&h.command)
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            spec.holder_match
                .iter()
                .any(|m| name.starts_with(&m.to_lowercase()))
        })
}

/// One service's part in the report.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceOutcome {
    pub name: String,
    pub stopped: bool,
    pub detail: String,
    /// Its configured stop line, when running it failed: the owner's Run
    /// card offers it (R4).
    #[serde(skip)]
    pub failed_stop: Option<String>,
}

/// The list in effect: the one read at start. The file's current list, when
/// it differs, is only reported.
pub fn changed_since_start(app: &AppState) -> bool {
    let path = app.cfg.home.join(crate::brand::daemon_file(".toml"));
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct File {
        quiesce: Option<QuiesceConfig>,
    }
    let now = toml::from_str::<File>(&text)
        .ok()
        .and_then(|f| f.quiesce)
        .unwrap_or_default();
    now.services != app.cfg.quiesce.services
}

/// Stops the services holding the home; records them on the pause.
pub fn stop_holding(app: &AppState, q: &Quiesce, holders: &[Holder]) -> Vec<ServiceOutcome> {
    let cfg = &app.cfg.quiesce;
    let mut stopped_names = Vec::new();
    let mut out = Vec::new();
    for spec in &cfg.services {
        let detect = run(&spec.detect, &cfg.search_path);
        let running = detect.ok && detect.output.contains(&spec.running_match);
        if !should_stop(spec, running, holders) {
            out.push(ServiceOutcome {
                name: spec.name.clone(),
                stopped: false,
                detail: if running {
                    "running, not holding the home"
                } else {
                    "not running"
                }
                .to_string(),
                failed_stop: None,
            });
            continue;
        }
        let stop = run(&spec.stop, &cfg.search_path);
        if stop.ok {
            stopped_names.push(spec.name.clone());
        }
        out.push(ServiceOutcome {
            name: spec.name.clone(),
            stopped: stop.ok,
            detail: stop.output.trim().chars().take(500).collect(),
            failed_stop: (!stop.ok).then(|| spec.stop.clone()),
        });
    }
    if let Err(e) = app.db.set_quiesce_services(&q.id, &stopped_names) {
        tracing::warn!(error = %e, "could not record the services quiesce stopped");
    }
    out
}

/// Starts again the services `q` stopped; returns their names.
pub fn restart_stopped(app: &AppState, q: &Quiesce) -> Vec<String> {
    let cfg = &app.cfg.quiesce;
    let mut started = Vec::new();
    for name in &q.services_stopped {
        let Some(spec) = cfg.services.iter().find(|s| &s.name == name) else {
            continue;
        };
        let ran = run(&spec.start, &cfg.search_path);
        if ran.ok {
            started.push(name.clone());
        } else {
            tracing::warn!(service = %name, output = %ran.output, "could not start a service again");
        }
    }
    started
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holder(command: &str) -> Holder {
        Holder {
            pid: 7,
            pgid: None,
            command: command.into(),
            cwd: false,
            path: PathBuf::from("/home"),
        }
    }

    #[test]
    fn a_service_is_stopped_only_while_it_runs_and_holds_the_home() {
        let mut spec = QuiesceConfig::default().services[0].clone();
        let lima = [holder("limactl")];
        let editor = [holder("Code Helper")];
        // running × holding × always
        for (running, holders, always, stop) in [
            (false, &lima[..], false, false),
            (false, &lima[..], true, false),
            (true, &lima[..], false, true),
            (true, &editor[..], false, false),
            (true, &[][..], false, false),
            (true, &[][..], true, true),
        ] {
            spec.always = always;
            assert_eq!(
                should_stop(&spec, running, holders),
                stop,
                "running {running}, holders {holders:?}, always {always}"
            );
        }
    }

    #[test]
    fn commands_run_by_absolute_path_never_through_path() {
        let dir = tempfile::tempdir().unwrap();
        // A `brew` only on PATH isn't found; one in the search path is.
        assert!(resolve("brew services stop colima", &[dir.path().to_path_buf()]).is_none());
        std::fs::write(dir.path().join("brew"), "").unwrap();
        let (program, args) =
            resolve("brew services stop colima", &[dir.path().to_path_buf()]).unwrap();
        assert_eq!(program, dir.path().join("brew"));
        assert_eq!(args, ["services", "stop", "colima"]);
        let ran = run("nonexistent-tool --x", &[dir.path().to_path_buf()]);
        assert!(
            !ran.ok && ran.output.contains("not found"),
            "{}",
            ran.output
        );
    }

    #[test]
    fn the_defaults_ship_colima_and_lima_looked_up_in_fixed_places() {
        let cfg = QuiesceConfig::default();
        let names: Vec<&str> = cfg.services.iter().map(|s| s.name.as_str()).collect();
        let expected: &[&str] = if cfg!(windows) {
            &[]
        } else {
            &["colima", "lima"]
        };
        assert_eq!(names, expected);
        assert!(cfg.search_path.iter().all(|p| p.is_absolute()));
        assert_eq!(cfg.deadline_minutes, 30);
    }
}
