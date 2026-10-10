//! Unpacking a staged build: the app bundle out of a zip or a disk image,
//! or the file itself (moved out of install.rs for the 400-line limit).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{run_ok, Kind};

/// What gets checked and installed, inside the stage: the app bundle out
/// of a zip or a disk image, or the file itself.
pub(super) fn unpack(kind: Kind, file: &Path, stage: &Path) -> anyhow::Result<PathBuf> {
    match kind {
        Kind::AppZip => {
            let unpacked = stage.join("unpacked");
            run_ok(
                Command::new("ditto")
                    .args(["-x", "-k"])
                    .arg(file)
                    .arg(&unpacked),
            )?;
            find_app(&unpacked)
        }
        Kind::AppDmg => {
            let mount = stage.join("mount");
            run_ok(
                Command::new("hdiutil")
                    .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
                    .arg(&mount)
                    .arg(file),
            )?;
            // A copy in the stage, so the image can go before the swap.
            let copied = find_app(&mount).and_then(|app| {
                let to = stage
                    .join("unpacked")
                    .join(app.file_name().unwrap_or_default());
                run_ok(Command::new("ditto").arg(&app).arg(&to)).map(|()| to)
            });
            let _ = Command::new("hdiutil").arg("detach").arg(&mount).status();
            copied
        }
        Kind::WindowsSetup => Ok(file.to_path_buf()),
        Kind::Daemon => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(file.to_path_buf())
        }
    }
}

fn find_app(dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| anyhow::anyhow!("no app bundle in {}", dir.display()))
}
