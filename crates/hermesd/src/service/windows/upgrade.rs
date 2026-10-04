//! Staging, swapping and rolling back the daemon binary.
use std::path::{Path, PathBuf};

use anyhow::Context;

use super::host::Host;
use super::task::write_task_files;
use super::{same_path, ServicePaths, DEFAULT_CONFIG};

pub(super) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

fn sha256_file(path: &Path) -> anyhow::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buffer[..n]);
    }
}

/// Fails unless `copy` holds exactly the bytes of `source`.
pub(super) fn verify_copy(source: &Path, copy: &Path) -> anyhow::Result<()> {
    let expected = std::fs::metadata(source)?.len();
    let actual = std::fs::metadata(copy)?.len();
    anyhow::ensure!(
        expected == actual,
        "staged binary is {actual} bytes, expected {expected}"
    );
    anyhow::ensure!(
        sha256_file(source)? == sha256_file(copy)?,
        "staged binary checksum differs from {}",
        source.display()
    );
    Ok(())
}

/// Copies `source` to `bin\hermesd.exe.new` and checks the copy, returning
/// its path and version. The running daemon is not touched, so a copy that
/// does not land (a full disk, a quarantine) costs nothing.
fn stage(
    source: &Path,
    paths: &ServicePaths,
    host: &impl Host,
) -> anyhow::Result<(PathBuf, String)> {
    let staged = with_suffix(&paths.bin_path(), ".new");
    let checked = (|| {
        remove_if_present(&staged)?;
        std::fs::copy(source, &staged).context("copying daemon binary")?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&staged)?
            .sync_all()?;
        verify_copy(source, &staged)?;
        host.version_of(&staged)
    })();
    match checked {
        Ok(version) => Ok((staged, version)),
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            Err(error.context("staged daemon binary failed verification; nothing was stopped"))
        }
    }
}

pub(super) fn remove_if_present(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Live files moved aside as `<name>.old` while a new install proves itself.
struct Backup {
    /// (live path, backup path, whether the live path existed)
    entries: Vec<(PathBuf, PathBuf, bool)>,
}

impl Backup {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Moves `live` aside; recorded even when absent so a restore removes
    /// whatever the install put there.
    fn keep(&mut self, live: PathBuf) -> anyhow::Result<()> {
        let old = with_suffix(&live, ".old");
        let existed = live.exists();
        if existed {
            remove_if_present(&old)?;
            std::fs::rename(&live, &old)
                .with_context(|| format!("moving {} aside", live.display()))?;
        }
        self.entries.push((live, old, existed));
        Ok(())
    }

    fn restore(&self) -> anyhow::Result<()> {
        for (live, old, existed) in self.entries.iter().rev() {
            if *existed {
                // std::fs::rename is MoveFileExW with MOVEFILE_REPLACE_EXISTING.
                std::fs::rename(old, live)
                    .with_context(|| format!("restoring {}", live.display()))?;
            } else {
                remove_if_present(live)?;
            }
        }
        Ok(())
    }

    fn discard(&self) -> anyhow::Result<()> {
        for (_, old, _) in &self.entries {
            remove_if_present(old)?;
        }
        Ok(())
    }
}

/// Installs `source` and starts it, or leaves the previous install running.
///
/// The order is what keeps a failed upgrade from leaving `bin` empty: the new
/// binary is staged and verified before anything is stopped; the old task is
/// disabled (RestartOnFailure would relaunch it) and its whole process tree
/// reaped; the binary is swapped by rename with the old one kept as `.old`;
/// and only a `/health` answer from the new version lets the backups and the
/// pre-rename binary go. Any failure after the stop restores them and
/// restarts the old task.
pub(super) fn upgrade(source: &Path, paths: &ServicePaths, host: &impl Host) -> anyhow::Result<()> {
    std::fs::create_dir_all(paths.home.join("bin"))?;
    std::fs::create_dir_all(paths.log_dir())?;
    if !paths.config_path().exists() {
        std::fs::write(paths.config_path(), DEFAULT_CONFIG)?;
    }
    let bin = paths.bin_path();
    let (staged, version) = if same_path(source, &bin) {
        (None, host.version_of(source)?)
    } else {
        let (staged, version) = stage(source, paths, host)?;
        (Some(staged), version)
    };
    let previous = paths.plist_path().is_file();
    if previous {
        if let Err(error) = host.disable().and_then(|()| host.stop()) {
            if let Some(staged) = &staged {
                let _ = std::fs::remove_file(staged);
            }
            // The old daemon may be half-stopped; put its task back as it was.
            let restarted = host.register().and_then(|()| host.start());
            return Err(match restarted {
                Ok(()) => error.context("could not stop the running daemon; it was left running"),
                Err(again) => error.context(format!(
                    "could not stop the running daemon, nor restart it: {again:#}"
                )),
            });
        }
    }
    // Stale from the stopped daemon; the health check must read the new one.
    remove_if_present(&crate::home::runtime_port_path(&paths.home))?;

    let mut backup = Backup::new();
    let swapped = (|| {
        if let Some(staged) = &staged {
            backup.keep(bin.clone())?;
            std::fs::rename(staged, &bin).context("swapping in the new daemon binary")?;
        }
        backup.keep(paths.launcher_path())?;
        backup.keep(paths.plist_path())?;
        write_task_files(paths)?;
        host.register()?;
        host.start()?;
        host.wait_healthy(&version)
    })();
    if let Err(error) = swapped {
        if let Some(staged) = &staged {
            let _ = std::fs::remove_file(staged);
        }
        return Err(match rollback(host, &backup, previous) {
            Ok(()) if previous => error.context("install failed; the previous daemon was restored"),
            Ok(()) => error.context("install failed and was undone"),
            Err(again) => {
                error.context(format!("install failed, and so did undoing it: {again:#}"))
            }
        });
    }
    backup.discard()?;
    remove_if_present(&paths.legacy_bin_path())
}

fn rollback(host: &impl Host, backup: &Backup, previous: bool) -> anyhow::Result<()> {
    // The new daemon must be gone before its binary can be moved back.
    let _ = host.disable();
    host.stop()?;
    backup.restore()?;
    if previous {
        host.register()?;
        host.start()
    } else {
        host.delete()
    }
}
