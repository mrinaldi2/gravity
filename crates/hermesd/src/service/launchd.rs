//! The launchd side of an install: which agents exist, stopping one with
//! everything it started, and loading one. `launchctl` itself sits behind
//! [`Launchctl`], so tests never call it.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Context;

use super::sequence::{Host, Identity};
use super::stage::remove_if_present;
use super::{render_plist, ServicePaths, LAUNCHD_LABEL};

/// The `launchctl` calls an install makes, plus the version and health
/// probes, which tests replace as well.
pub(super) trait Launchctl {
    /// The PID launchd reports for the loaded agent `label`, if it runs.
    fn pid(&self, label: &str) -> Option<u32>;
    /// Unloads the agent; returns once its process has exited. An agent
    /// that is not loaded is not an error.
    fn bootout(&self, plist: &Path) -> anyhow::Result<()>;
    /// Loads the agent, which starts it (`RunAtLoad`).
    fn bootstrap(&self, plist: &Path) -> anyhow::Result<()>;
    /// Whether launchd has the agent `label` loaded, running or not.
    fn is_loaded(&self, label: &str) -> bool;
    fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
        super::stage::version_of(binary)
    }
    fn wait_healthy(&self, home: &Path, port: u16, version: &str) -> anyhow::Result<()> {
        super::sequence::wait_healthy(home, port, version)
    }
}

/// The real `launchctl`, in the user's GUI domain.
pub(super) struct System;

fn gui_domain() -> anyhow::Result<String> {
    let out = Command::new("id")
        .arg("-u")
        .output()
        .context("running id -u")?;
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    anyhow::ensure!(!uid.is_empty(), "could not determine uid");
    Ok(format!("gui/{uid}"))
}

impl Launchctl for System {
    fn pid(&self, label: &str) -> Option<u32> {
        let target = format!("{}/{label}", gui_domain().ok()?);
        let out = Command::new("launchctl")
            .args(["print", &target])
            .output()
            .ok()?;
        parse_agent_pid(&String::from_utf8_lossy(&out.stdout))
    }

    fn is_loaded(&self, label: &str) -> bool {
        let Ok(domain) = gui_domain() else {
            return false;
        };
        Command::new("launchctl")
            .args(["print", &format!("{domain}/{label}")])
            .output()
            .is_ok_and(|out| out.status.success())
    }

    fn bootout(&self, plist: &Path) -> anyhow::Result<()> {
        let domain = gui_domain()?;
        let out = Command::new("launchctl")
            .args(["bootout", &domain])
            .arg(plist)
            .output()
            .context("running launchctl bootout")?;
        // A failure here is normal when nothing is loaded yet.
        tracing::info!(
            %domain,
            plist = %plist.display(),
            status = out.status.code().unwrap_or(-1),
            stderr = %String::from_utf8_lossy(&out.stderr).trim(),
            "launchctl bootout"
        );
        Ok(())
    }

    fn bootstrap(&self, plist: &Path) -> anyhow::Result<()> {
        let domain = gui_domain()?;
        let out = Command::new("launchctl")
            .args(["bootstrap", &domain])
            .arg(plist)
            .output()
            .context("running launchctl bootstrap")?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::ensure!(
            out.status.success(),
            "launchctl bootstrap {} failed: {}",
            plist.display(),
            stderr.trim()
        );
        Ok(())
    }
}

pub(super) fn parse_agent_pid(print: &str) -> Option<u32> {
    print
        .lines()
        .find_map(|line| line.trim().strip_prefix("pid = "))
        .and_then(|pid| pid.trim().parse().ok())
}

/// The agents an install moves between: the pre-rename one, whose daemon
/// runs from `old_home`, and the current one in `paths`' home.
pub(super) struct Launchd<'a, L: Launchctl> {
    pub(super) paths: &'a ServicePaths,
    pub(super) old_home: PathBuf,
    pub(super) port: u16,
    pub(super) launchctl: L,
}

impl<L: Launchctl> Launchd<'_, L> {
    fn plist(&self, id: Identity) -> PathBuf {
        match id {
            Identity::Legacy => self.paths.legacy_plist_path(),
            Identity::Current => self.paths.plist_path(),
        }
    }

    /// The homes `id`'s daemon may hold.
    fn homes(&self, id: Identity) -> Vec<PathBuf> {
        let mut homes = match id {
            Identity::Legacy => vec![
                self.old_home.clone(),
                self.paths
                    .user_home
                    .join(crate::brand::LEGACY_HOME_DIR_NAME),
            ],
            Identity::Current => vec![self.paths.home.clone()],
        };
        homes.dedup();
        homes.retain(|home| home.is_dir());
        homes
    }

    /// Bot sessions do not die with the daemon: a detached child (an MCP
    /// server, a browser) outlives the terminal hangup, and the migration
    /// would find it holding the home. Groups come from the daemon's process
    /// tree before the bootout and from the sessions it recorded (a 0.14
    /// daemon records none); only those still holding the home are stopped.
    fn stop_sessions(&self, homes: &[PathBuf], mut groups: std::collections::BTreeSet<u32>) {
        for home in homes {
            groups.extend(crate::holders::recorded_sessions(home));
            match crate::holders::stop_owned(home, &groups) {
                Ok(stopped) if !stopped.is_empty() => {
                    tracing::info!(?stopped, home = %home.display(), "stopped bot session groups")
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "could not stop bot session groups"),
            }
        }
    }
}

impl<L: Launchctl> Host for Launchd<'_, L> {
    fn installed(&self) -> Vec<Identity> {
        [Identity::Legacy, Identity::Current]
            .into_iter()
            .filter(|id| self.plist(*id).is_file())
            .collect()
    }

    fn disable(&self, _id: Identity) -> anyhow::Result<()> {
        // `bootout` unloads the agent, so KeepAlive cannot relaunch it, and
        // its plist stays for a rollback to load again.
        Ok(())
    }

    fn stop(&self, id: Identity) -> anyhow::Result<()> {
        let label = label(id);
        let homes = self.homes(id);
        let groups = self
            .launchctl
            .pid(label)
            .map(crate::holders::descendant_groups)
            .unwrap_or_default();
        self.launchctl.bootout(&self.plist(id))?;
        self.stop_sessions(&homes, groups);
        // A daemon launchd no longer tracks (an earlier bootout that timed
        // out, a hand-started copy) still runs the managed binary.
        let binaries: Vec<PathBuf> = homes
            .iter()
            .flat_map(|home| {
                let bin = home.join("bin");
                [bin.join("hermesd"), bin.join("gravityd")]
            })
            .collect();
        super::reap::reap(&binaries)?;
        for home in &homes {
            wait_released(home)?;
        }
        Ok(())
    }

    fn definition_files(&self) -> Vec<PathBuf> {
        vec![self.paths.plist_path()]
    }

    fn write_definition(&self) -> anyhow::Result<()> {
        let plist = self.paths.plist_path();
        std::fs::create_dir_all(&self.paths.launch_agents)?;
        std::fs::write(
            &plist,
            render_plist(
                &self.paths.bin_path(),
                &self.paths.log_dir(),
                &self.paths.user_home,
            ),
        )
        .with_context(|| format!("writing {}", plist.display()))
    }

    fn start(&self, id: Identity) -> anyhow::Result<()> {
        // An agent whose bootout failed is still loaded; bootstrapping it
        // again would fail although it is where a rollback wants it.
        if self.launchctl.is_loaded(label(id)) {
            return Ok(());
        }
        self.launchctl.bootstrap(&self.plist(id))
    }

    fn remove(&self, id: Identity) -> anyhow::Result<()> {
        let plist = self.plist(id);
        if plist.is_file() {
            self.launchctl.bootout(&plist)?;
        }
        remove_if_present(&plist)
    }

    fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
        self.launchctl.version_of(binary)
    }

    fn wait_healthy(&self, version: &str) -> anyhow::Result<()> {
        self.launchctl
            .wait_healthy(&self.paths.home, self.port, version)
    }
}

fn label(id: Identity) -> &'static str {
    match id {
        Identity::Legacy => crate::brand::LEGACY_LAUNCHD_LABEL,
        Identity::Current => LAUNCHD_LABEL,
    }
}

/// Waits for the stopped daemon to let go of `home`'s lock.
fn wait_released(home: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match crate::home::lock(home) {
            Ok(_lock) => return Ok(()),
            Err(error) if Instant::now() >= deadline => {
                return Err(error).context("waiting for the managed daemon to stop")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}
