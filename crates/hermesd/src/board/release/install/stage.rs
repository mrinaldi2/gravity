//! Where a build waits while it is checked (ARCH-R43 M2): a fresh folder
//! only this user can open, made for this install and never reused. The
//! build is copied or downloaded into it and its sha256 is checked there,
//! so what gets installed is the very file that was checked.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use super::{run_ok, Build};

pub(crate) struct Stage {
    pub dir: PathBuf,
}

impl Stage {
    /// A new private folder under the system's temporary one: created, not
    /// reused, so nothing another process left there can be picked up.
    pub fn new(release: &str) -> anyhow::Result<Self> {
        let name: String = release
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let dir = std::env::temp_dir().join(format!(
            "hermes-install-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir)?;
        Ok(Self { dir })
    }

    /// The build's file inside the stage: copied from the home's own file
    /// when this is the home, else downloaded from its HTTPS url.
    pub fn fetch(&self, build: &Build) -> anyhow::Result<PathBuf> {
        if let Some(local) = build.artifact.as_deref().map(Path::new) {
            if local.is_file() {
                let file = self.dir.join(local.file_name().unwrap_or_default());
                std::fs::copy(local, &file)?;
                return Ok(file);
            }
        }
        let url = build
            .url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("{} has no file here and no url", build.platform))?;
        let name = url
            .rsplit('/')
            .next()
            .filter(|n| !n.is_empty() && !n.contains(['\\', ':']) && *n != "..")
            .unwrap_or("build");
        let file = self.dir.join(name);
        run_ok(
            Command::new("curl")
                .args(["-fsSL", "--proto", "=https", "--retry", "2", "-o"])
                .arg(&file)
                .arg(url),
        )?;
        Ok(file)
    }

    /// Removes the stage, for a dry run or an install that stopped. A handed
    /// off install removes it itself when it's done.
    pub fn remove(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The file is the build the owner approved: its sha256 is the frozen one.
pub(crate) fn verify(file: &Path, sha256: &str) -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(file)?, &mut hasher)?;
    let got = hex::encode(hasher.finalize());
    anyhow::ensure!(
        !sha256.is_empty() && got == sha256,
        "{} doesn't match the release: sha256 {got}, expected {sha256}; not installing it",
        file.display()
    );
    Ok(())
}
