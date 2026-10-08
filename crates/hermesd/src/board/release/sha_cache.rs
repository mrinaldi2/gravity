//! A served build's sha256, remembered while its file stays the same (CE
//! review of H-229): `release_install` checks the IPA on every call, and an
//! IPA is large. The key is the canonical path the caller resolved; the
//! file is hashed again whenever its size, modification time, device, inode
//! or status-change time changes (CE review of H-241, M1). A writer can pad
//! a swapped file to the same size and set its mtime back, but it can't set
//! the ctime, and a file renamed into place is another inode. Off Unix the
//! standard library gives no change time, so nothing is cached there: every
//! call hashes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use super::serve;

/// The served builds' hashes, for the life of the daemon.
pub static SHAS: ShaCache = ShaCache::new();

/// What a file looked like when it was hashed.
#[cfg_attr(not(unix), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stamp {
    size: u64,
    modified: SystemTime,
    device: u64,
    inode: u64,
    changed: (i64, i64),
}

/// A regular file's stamp; nothing for a symlink, a folder or a missing one.
#[cfg(unix)]
fn stamp(path: &Path) -> Option<Stamp> {
    use std::os::unix::fs::MetadataExt;

    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(Stamp {
        size: meta.len(),
        modified: meta.modified().ok()?,
        device: meta.dev(),
        inode: meta.ino(),
        changed: (meta.ctime(), meta.ctime_nsec()),
    })
}

pub struct ShaCache(Mutex<BTreeMap<PathBuf, (Stamp, String)>>);

impl Default for ShaCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ShaCache {
    pub const fn new() -> Self {
        ShaCache(Mutex::new(BTreeMap::new()))
    }

    /// Whether `path` is a regular file that hashes to `sha256`, as
    /// `serve::matches` says, hashing only when the file changed.
    pub fn matches(&self, path: &Path, sha256: &str) -> bool {
        self.sha256(path, serve::sha256_file)
            .is_some_and(|actual| actual == sha256)
    }

    /// No change time to trust: a regular file is hashed every time.
    #[cfg(not(unix))]
    fn sha256(
        &self,
        path: &Path,
        hash: impl FnOnce(&Path) -> anyhow::Result<String>,
    ) -> Option<String> {
        let meta = fs::symlink_metadata(path).ok()?;
        meta.is_file().then(|| hash(path).ok()).flatten()
    }

    #[cfg(unix)]
    fn sha256(
        &self,
        path: &Path,
        hash: impl FnOnce(&Path) -> anyhow::Result<String>,
    ) -> Option<String> {
        let before = stamp(path)?;
        if let Some((seen, sha)) = self.lock().get(path) {
            if *seen == before {
                return Some(sha.clone());
            }
        }
        // Hashed without the lock: an IPA takes a while, and other builds
        // may be asked about meanwhile.
        let sha = hash(path).ok();
        let mut map = self.lock();
        // A file that changed while it was hashed isn't remembered, so the
        // next call hashes it again.
        match &sha {
            Some(sha) if stamp(path) == Some(before) => {
                map.insert(path.to_path_buf(), (before, sha.clone()));
            }
            _ => {
                map.remove(path);
            }
        }
        sha
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<PathBuf, (Stamp, String)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    #[cfg(unix)]
    use std::time::Duration;

    use super::*;

    /// Asks the cache for `path`, counting the times it really hashed.
    fn ask(cache: &ShaCache, path: &Path, hashed: &Cell<u32>) -> Option<String> {
        cache.sha256(path, |p| {
            hashed.set(hashed.get() + 1);
            serve::sha256_file(p)
        })
    }

    /// Sets `path`'s modification time back to `to`.
    fn set_mtime(path: &Path, to: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(to)
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn hashes_once_until_the_file_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let ipa = tmp.path().join("TheHermes.ipa");
        fs::write(&ipa, "ipa bytes").unwrap();
        let cache = ShaCache::new();
        let hashed = Cell::new(0);
        let first = ask(&cache, &ipa, &hashed);
        assert_eq!(first.as_deref(), serve::sha256_file(&ipa).ok().as_deref());
        assert_eq!(ask(&cache, &ipa, &hashed), first);
        assert_eq!(ask(&cache, &ipa, &hashed), first);
        assert_eq!(hashed.get(), 1, "the unchanged file is hashed once");

        // Another size: hashed again, and the new sha is the answer.
        fs::write(&ipa, "other ipa bytes").unwrap();
        let second = ask(&cache, &ipa, &hashed);
        assert_eq!(hashed.get(), 2);
        assert_ne!(second, first);
        assert_eq!(second.as_deref(), serve::sha256_file(&ipa).ok().as_deref());

        // The same size, another mtime: hashed again.
        fs::write(&ipa, "OTHER IPA BYTES").unwrap();
        set_mtime(&ipa, SystemTime::now() + Duration::from_secs(60));
        let third = ask(&cache, &ipa, &hashed);
        assert_eq!(hashed.get(), 3);
        assert_eq!(third.as_deref(), serve::sha256_file(&ipa).ok().as_deref());
        assert_ne!(third, second);
        assert_eq!(ask(&cache, &ipa, &hashed), third);
        assert_eq!(hashed.get(), 3);
    }

    /// Another file of the same size with the old mtime set back (CE review
    /// of H-241, M1): overwritten in place its ctime moved, renamed into
    /// place it is another inode. Either way it is hashed again.
    #[cfg(unix)]
    #[test]
    fn a_same_size_swap_with_the_old_mtime_is_hashed_again() {
        let tmp = tempfile::tempdir().unwrap();
        let ipa = tmp.path().join("TheHermes.ipa");
        fs::write(&ipa, "ipa bytes").unwrap();
        let mtime = fs::metadata(&ipa).unwrap().modified().unwrap();
        let sha = serve::sha256_file(&ipa).unwrap();
        let cache = ShaCache::new();
        let hashed = Cell::new(0);
        assert_eq!(ask(&cache, &ipa, &hashed).as_deref(), Some(sha.as_str()));

        fs::write(&ipa, "IPA BYTES").unwrap();
        set_mtime(&ipa, mtime);
        assert_eq!(fs::metadata(&ipa).unwrap().modified().unwrap(), mtime);
        let in_place = ask(&cache, &ipa, &hashed);
        assert_eq!(hashed.get(), 2, "overwritten in place");
        assert_ne!(in_place.as_deref(), Some(sha.as_str()));
        assert!(!cache.matches(&ipa, &sha));

        let swap = tmp.path().join("swap.ipa");
        fs::write(&ipa, "ipa bytes").unwrap();
        assert!(cache.matches(&ipa, &sha), "the original bytes again");
        fs::write(&swap, "IPA bytes").unwrap();
        set_mtime(&swap, fs::metadata(&ipa).unwrap().modified().unwrap());
        fs::rename(&swap, &ipa).unwrap();
        let renamed = ask(&cache, &ipa, &hashed);
        assert_eq!(hashed.get(), 3, "renamed into place");
        assert_ne!(renamed.as_deref(), Some(sha.as_str()));
        assert!(!cache.matches(&ipa, &sha));
    }

    #[cfg(not(unix))]
    #[test]
    fn hashes_every_time_without_a_change_time() {
        let tmp = tempfile::tempdir().unwrap();
        let ipa = tmp.path().join("TheHermes.ipa");
        fs::write(&ipa, "ipa bytes").unwrap();
        let cache = ShaCache::new();
        let hashed = Cell::new(0);
        let first = ask(&cache, &ipa, &hashed);
        assert_eq!(ask(&cache, &ipa, &hashed), first);
        assert_eq!(hashed.get(), 2);
    }

    #[test]
    fn matches_like_serve_and_refuses_what_isnt_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let ipa = tmp.path().join("TheHermes.ipa");
        fs::write(&ipa, "ipa bytes").unwrap();
        let sha = serve::sha256_file(&ipa).unwrap();
        let cache = ShaCache::new();
        assert!(cache.matches(&ipa, &sha));
        assert!(!cache.matches(&ipa, &"b".repeat(64)));
        assert!(!cache.matches(tmp.path(), &sha), "a folder");
        assert!(!cache.matches(&tmp.path().join("gone.ipa"), &sha));
        fs::remove_file(&ipa).unwrap();
        assert!(!cache.matches(&ipa, &sha), "removed after it was cached");
        #[cfg(unix)]
        {
            let real = tmp.path().join("real.ipa");
            fs::write(&real, "ipa bytes").unwrap();
            std::os::unix::fs::symlink(&real, &ipa).unwrap();
            assert!(!cache.matches(&ipa, &sha), "a symlink");
        }
    }
}
