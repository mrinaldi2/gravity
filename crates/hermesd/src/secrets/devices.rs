//! Device credentials kept as sha256 hashes (H-044 T5, H-012 "hashed
//! tokens"). The phone holds the only plaintext, so copying the secrets
//! folder no longer gives a bot an approve-grant device.
//!
//! A newly paired device is stored as `device-<id>.sha256`. In phase 1 it
//! also gets the plaintext `device-<id>.token` an older daemon reads, so a
//! downgrade never strands a phone; phase 2 hashes any plaintext and deletes
//! it at start, and writes only the hash.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{constant_time_eq, random_token, read_token, remove_if_present, write_secret, Secrets};

fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn hash_file(dir: &Path, device_id: &str) -> std::path::PathBuf {
    dir.join(format!("device-{device_id}.sha256"))
}

fn token_file(dir: &Path, device_id: &str) -> std::path::PathBuf {
    dir.join(format!("device-{device_id}.token"))
}

/// Every device's hash -> id. With `enforce`, plaintext files become hashes.
pub(super) fn load(dir: &Path, enforce: bool) -> anyhow::Result<HashMap<String, String>> {
    let mut hashes = HashMap::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix("device-") else {
            continue;
        };
        if let Some(id) = rest.strip_suffix(".sha256") {
            if let Some(h) = read_token(&entry.path())? {
                hashes.insert(h, id.to_string());
            }
        } else if let Some(id) = rest.strip_suffix(".token") {
            if let Some(token) = read_token(&entry.path())? {
                let h = hash(&token);
                if enforce {
                    write_secret(&hash_file(dir, id), &h)?;
                    remove_if_present(&entry.path())?;
                }
                hashes.insert(h, id.to_string());
            }
        }
    }
    Ok(hashes)
}

impl Secrets {
    /// Issue (or replace) the credential for a device. The token is returned
    /// once; in phase 2 only its hash persists.
    pub fn issue_device_token(&self, device_id: &str) -> anyhow::Result<String> {
        let mut map = self.device_hashes.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, id| id.as_str() != device_id);
        let token = random_token();
        let h = hash(&token);
        write_secret(&hash_file(&self.dir, device_id), &h)?;
        if self.enforce {
            remove_if_present(&token_file(&self.dir, device_id))?;
        } else {
            write_secret(&token_file(&self.dir, device_id), &token)?;
        }
        map.insert(h, device_id.to_string());
        Ok(token)
    }

    pub fn device_for_token(&self, token: &str) -> Option<String> {
        let presented = hash(token);
        let map = self.device_hashes.lock().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .find(|(h, _)| constant_time_eq(h.as_bytes(), presented.as_bytes()))
            .map(|(_, id)| id.clone())
    }

    /// Delete a revoked device's credential so it can never reconnect.
    pub fn remove_device_token(&self, device_id: &str) -> anyhow::Result<()> {
        let mut map = self.device_hashes.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, id| id.as_str() != device_id);
        remove_if_present(&hash_file(&self.dir, device_id))?;
        remove_if_present(&token_file(&self.dir, device_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::Storage;

    fn enforced() -> Storage {
        Storage {
            enforce: true,
            ..Storage::default()
        }
    }

    #[test]
    fn a_device_is_stored_as_a_hash_and_still_recognised() {
        let dir = tempfile::tempdir().expect("dir");
        let secrets = Secrets::open(dir.path()).expect("open");
        let token = secrets.issue_device_token("d1").expect("issue");
        let stored = fs::read_to_string(hash_file(dir.path(), "d1")).expect("hash file");
        assert_ne!(stored, token);
        assert_eq!(
            fs::read_to_string(token_file(dir.path(), "d1")).expect("plaintext"),
            token,
            "phase 1 keeps what an older daemon reads"
        );
        assert_eq!(secrets.device_for_token(&token).as_deref(), Some("d1"));
        assert_eq!(
            secrets.device_for_token(&stored),
            None,
            "a copied hash is no token"
        );

        let reopened = Secrets::open(dir.path()).expect("reopen");
        assert_eq!(reopened.device_for_token(&token).as_deref(), Some("d1"));
        reopened.remove_device_token("d1").expect("remove");
        assert_eq!(reopened.device_for_token(&token), None);
        assert!(!hash_file(dir.path(), "d1").exists());
        assert!(!token_file(dir.path(), "d1").exists());
    }

    #[test]
    fn phase_two_pairs_devices_by_hash_only_and_drops_phase_one_plaintext() {
        let dir = tempfile::tempdir().expect("dir");
        let phase1 = Secrets::open(dir.path()).expect("phase 1");
        let early = phase1.issue_device_token("d1").expect("issue");
        assert!(token_file(dir.path(), "d1").exists());

        let phase2 = Secrets::open_for(dir.path(), enforced()).expect("phase 2");
        assert!(!token_file(dir.path(), "d1").exists(), "deleted at start");
        assert_eq!(phase2.device_for_token(&early).as_deref(), Some("d1"));
        let late = phase2.issue_device_token("d2").expect("issue");
        assert!(hash_file(dir.path(), "d2").exists());
        assert!(!token_file(dir.path(), "d2").exists(), "never written");
        assert_eq!(phase2.device_for_token(&late).as_deref(), Some("d2"));
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .expect("list")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files renamed away: {leftovers:?}"
        );
    }

    #[test]
    fn phase_two_hashes_old_device_files_and_deletes_owner_and_bot_tokens() {
        let dir = tempfile::tempdir().expect("dir");
        let old = Secrets::open(dir.path()).expect("phase 1");
        let bot = old.bot_token("b1").expect("bot");
        let client = old.client_token().to_string();
        fs::write(token_file(dir.path(), "d0"), "phone-secret").expect("old device");

        let phase1 = Secrets::open(dir.path()).expect("phase 1 again");
        assert_eq!(
            phase1.device_for_token("phone-secret").as_deref(),
            Some("d0")
        );
        assert!(
            token_file(dir.path(), "d0").exists(),
            "phase 1 changes nothing"
        );

        let phase2 = Secrets::open_for(dir.path(), enforced()).expect("phase 2");
        assert!(!dir.path().join("client.token").exists());
        assert!(!dir.path().join("bot-b1.token").exists());
        assert!(!token_file(dir.path(), "d0").exists());
        assert!(!phase2.verify_client(&client));
        assert!(
            !phase2.verify_client(""),
            "no client token means none matches"
        );
        assert_eq!(phase2.bot_for_token(&bot), None);
        assert_eq!(
            phase2.device_for_token("phone-secret").as_deref(),
            Some("d0")
        );

        // A bot started in phase 2 gets a token in memory only.
        phase2.bot_token("b2").expect("bot");
        assert!(!dir.path().join("bot-b2.token").exists());
        let again = Secrets::open_for(dir.path(), enforced()).expect("restart");
        assert!(!dir.path().join("client.token").exists(), "never recreated");
        assert_eq!(
            again.device_for_token("phone-secret").as_deref(),
            Some("d0")
        );
    }
}
