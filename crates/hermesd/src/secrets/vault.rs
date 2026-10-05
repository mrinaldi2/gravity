//! Where peer link tokens are kept (H-044 T5). A peer token lets whoever holds
//! it drive the peer daemon, so on macOS it can live in the login Keychain,
//! whose item ACL trusts only the binary that created it, instead of in a
//! file every bot can read.
//!
//! `[auth] peer_tokens = "keychain"` turns that on; the default stays
//! `"file"`. hermesd is ad-hoc signed, so the ACL pins one build: after an
//! update macOS asks the owner once per item before the daemon may read it.
//! It is meant to be switched on once the app and daemon are code-signed.
//! On Windows and Linux the setting falls back to files (DPAPI is user-wide:
//! a known residual).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{read_token, remove_if_present, write_secret};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerTokens {
    /// `secrets/peer-<id>.token`.
    #[default]
    File,
    /// The macOS login Keychain; `peer-*.token` files are moved there.
    Keychain,
}

pub(super) enum Vault {
    File(PathBuf),
    #[cfg(target_os = "macos")]
    Keychain(keychain::Keychain),
}

fn file(dir: &Path, peer_id: &str) -> PathBuf {
    dir.join(format!("peer-{peer_id}.token"))
}

fn files(dir: &Path) -> anyhow::Result<HashMap<String, String>> {
    let mut tokens = HashMap::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(peer_id) = name
            .strip_prefix("peer-")
            .and_then(|s| s.strip_suffix(".token"))
        {
            if let Some(token) = read_token(&entry.path())? {
                tokens.insert(token, peer_id.to_string());
            }
        }
    }
    Ok(tokens)
}

impl Vault {
    /// Opens the store and returns every token in it (token -> peer id).
    pub(super) fn open(
        dir: &Path,
        mode: PeerTokens,
    ) -> anyhow::Result<(Self, HashMap<String, String>)> {
        match mode {
            PeerTokens::File => Ok((Self::File(dir.to_path_buf()), files(dir)?)),
            #[cfg(target_os = "macos")]
            PeerTokens::Keychain => {
                let vault = keychain::Keychain::for_home(dir);
                // Move files over first, so a token is never in neither place.
                for (token, peer_id) in files(dir)? {
                    vault.put(&peer_id, &token)?;
                    remove_if_present(&file(dir, &peer_id))?;
                }
                let tokens = vault.all()?;
                Ok((Self::Keychain(vault), tokens))
            }
            #[cfg(not(target_os = "macos"))]
            PeerTokens::Keychain => {
                tracing::warn!(
                    "peer_tokens = \"keychain\" is macOS only; keeping peer tokens in files"
                );
                Ok((Self::File(dir.to_path_buf()), files(dir)?))
            }
        }
    }

    pub(super) fn put(&self, peer_id: &str, token: &str) -> anyhow::Result<()> {
        match self {
            Self::File(dir) => write_secret(&file(dir, peer_id), token),
            #[cfg(target_os = "macos")]
            Self::Keychain(k) => k.put(peer_id, token),
        }
    }

    pub(super) fn remove(&self, peer_id: &str) -> anyhow::Result<()> {
        match self {
            Self::File(dir) => remove_if_present(&file(dir, peer_id)),
            #[cfg(target_os = "macos")]
            Self::Keychain(k) => k.remove(peer_id),
        }
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use std::collections::HashMap;
    use std::path::Path;

    use security_framework::base::Error;
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
    use security_framework::passwords;
    use sha2::{Digest, Sha256};

    /// `errSecItemNotFound`.
    const NOT_FOUND: i32 = -25300;

    /// Generic passwords under one service per home, so a test or dev home
    /// never sees the real daemon's links. The account is the peer id.
    pub struct Keychain {
        service: String,
    }

    impl Keychain {
        pub fn for_home(secrets_dir: &Path) -> Self {
            let digest = Sha256::digest(secrets_dir.to_string_lossy().as_bytes());
            Self {
                service: format!("The Hermes peer link {}", &hex::encode(digest)[..12]),
            }
        }

        pub fn put(&self, peer_id: &str, token: &str) -> anyhow::Result<()> {
            passwords::set_generic_password(&self.service, peer_id, token.as_bytes())
                .map_err(|e| anyhow::anyhow!("storing peer {peer_id}'s token in the Keychain: {e}"))
        }

        pub fn remove(&self, peer_id: &str) -> anyhow::Result<()> {
            match passwords::delete_generic_password(&self.service, peer_id) {
                Err(e) if e.code() != NOT_FOUND => Err(anyhow::anyhow!(
                    "removing peer {peer_id}'s token from the Keychain: {e}"
                )),
                _ => Ok(()),
            }
        }

        /// token -> peer id for every link this home keeps.
        pub fn all(&self) -> anyhow::Result<HashMap<String, String>> {
            let found = ItemSearchOptions::new()
                .class(ItemClass::generic_password())
                .service(&self.service)
                .load_attributes(true)
                .limit(Limit::All)
                .search();
            let items = match found {
                Err(e) if e.code() == NOT_FOUND => return Ok(HashMap::new()),
                other => other.map_err(|e: Error| anyhow::anyhow!("reading the Keychain: {e}"))?,
            };
            let mut tokens = HashMap::new();
            for item in items {
                let Some(peer_id) = item.simplify_dict().and_then(|d| d.get("acct").cloned())
                else {
                    continue;
                };
                let token = passwords::get_generic_password(&self.service, &peer_id)
                    .map_err(|e| anyhow::anyhow!("reading peer {peer_id}'s token: {e}"))?;
                tokens.insert(String::from_utf8_lossy(&token).into_owned(), peer_id);
            }
            Ok(tokens)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Touches the real login Keychain, so it runs only when asked:
        /// `cargo test -p hermesd keychain -- --ignored`.
        #[test]
        #[ignore = "uses the login Keychain"]
        fn peer_tokens_round_trip_through_the_keychain() {
            let dir = tempfile::tempdir().expect("dir");
            let k = Keychain::for_home(dir.path());
            k.put("p1", "tok-1").expect("put");
            k.put("p1", "tok-2").expect("replace");
            assert_eq!(
                k.all().expect("all").get("tok-2").map(String::as_str),
                Some("p1")
            );
            assert!(!k.all().expect("all").contains_key("tok-1"));
            k.remove("p1").expect("remove");
            k.remove("p1").expect("already gone");
            assert!(k.all().expect("all").is_empty());
        }

        #[test]
        #[ignore = "uses the login Keychain"]
        fn switching_to_the_keychain_moves_peer_files_into_it() {
            let dir = tempfile::tempdir().expect("dir");
            std::fs::write(dir.path().join("peer-p2.token"), "link-2").expect("file");
            let secrets = crate::secrets::Secrets::open_for(
                dir.path(),
                crate::secrets::Storage {
                    enforce: false,
                    peer_tokens: super::super::PeerTokens::Keychain,
                },
            )
            .expect("open");
            assert!(!dir.path().join("peer-p2.token").exists(), "file moved");
            assert_eq!(secrets.peer_for_token("link-2").as_deref(), Some("p2"));
            secrets.store_peer_token("p3", "link-3").expect("store");
            assert!(!dir.path().join("peer-p3.token").exists(), "never written");
            for peer in ["p2", "p3"] {
                secrets.remove_peer_token(peer).expect("remove");
            }
            assert!(Keychain::for_home(dir.path())
                .all()
                .expect("all")
                .is_empty());
        }
    }
}
