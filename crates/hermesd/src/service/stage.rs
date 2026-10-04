//! Staging a new daemon binary beside the live one, checking it, and keeping
//! the files an install replaces as `.old` until the new daemon is healthy.
//! Shared by the launchd and Task Scheduler installs.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;

const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

pub(crate) fn remove_if_present(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Whether two spellings name the same file; Windows paths ignore case and
/// separator.
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let normal = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
        normal(a) == normal(b)
    } else {
        a == b
    }
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
pub(crate) fn verify_copy(source: &Path, copy: &Path) -> anyhow::Result<()> {
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

/// Copies `source` to `<live>.new` and checks the copy: size, checksum and
/// `--version` (through `version_of`). Returns the staged path and version.
/// Nothing running is touched, so a copy that does not land (a full disk, a
/// quarantine) costs nothing; on failure the `.new` file is removed.
pub(crate) fn stage(
    source: &Path,
    live: &Path,
    version_of: impl Fn(&Path) -> anyhow::Result<String>,
) -> anyhow::Result<(PathBuf, String)> {
    let staged = with_suffix(live, ".new");
    let checked = (|| {
        if let Some(dir) = live.parent() {
            std::fs::create_dir_all(dir)?;
        }
        remove_if_present(&staged)?;
        std::fs::copy(source, &staged).context("copying daemon binary")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(&staged)?
            .sync_all()?;
        verify_copy(source, &staged)?;
        version_of(&staged)
    })();
    match checked {
        Ok(version) => Ok((staged, version)),
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            Err(error.context("staged daemon binary failed verification; nothing was stopped"))
        }
    }
}

/// What `binary --version` reports, waiting at most 15 s for it.
pub(crate) fn version_of(binary: &Path) -> anyhow::Result<String> {
    let mut command = Command::new(binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("running {} --version", binary.display()))?;
    let deadline = Instant::now() + VERSION_TIMEOUT;
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("{} --version did not exit", binary.display());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output()?;
    anyhow::ensure!(
        out.status.success(),
        "{} --version failed",
        binary.display()
    );
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// The version in `hermesd --version` output (`hermesd 0.15.0`).
pub(crate) fn parse_version(output: &str) -> anyhow::Result<String> {
    let version = output
        .split_whitespace()
        .last()
        .context("the binary reported no version")?;
    anyhow::ensure!(
        version.starts_with(|c: char| c.is_ascii_digit()),
        "unexpected version output: {}",
        output.trim()
    );
    Ok(version.to_string())
}

/// Live files moved aside as `<name>.old` while a new install proves itself.
#[derive(Default)]
pub(crate) struct Backup {
    /// (live path, backup path, whether the live path existed)
    entries: Vec<(PathBuf, PathBuf, bool)>,
}

impl Backup {
    /// Moves `live` aside; recorded even when absent so a restore removes
    /// whatever the install put there.
    pub(crate) fn keep(&mut self, live: PathBuf) -> anyhow::Result<()> {
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

    /// Puts every kept file back, newest first, and removes what the install
    /// wrote where nothing was.
    pub(crate) fn restore(&self) -> anyhow::Result<()> {
        for (live, old, existed) in self.entries.iter().rev() {
            if *existed {
                // A rename replaces the destination (MoveFileExW with
                // MOVEFILE_REPLACE_EXISTING on Windows).
                std::fs::rename(old, live)
                    .with_context(|| format!("restoring {}", live.display()))?;
            } else {
                remove_if_present(live)?;
            }
        }
        Ok(())
    }

    pub(crate) fn discard(&self) -> anyhow::Result<()> {
        for (_, old, _) in &self.entries {
            remove_if_present(old)?;
        }
        Ok(())
    }
}
