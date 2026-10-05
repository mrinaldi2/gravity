//! Token management: one client token for the control plane, one scoped token
//! per bot for the MCP bridge. Files live under `~/.thehermes/secrets` with
//! mode 0700/0600 and are never exported.
//!
//! Phase 2 of H-044 (`[auth] bot_bearer = "refuse"`) keeps no owner or bot
//! token on disk at all: `client.token` and `bot-*.token` are deleted at
//! start and never written again, and device tokens are kept only as hashes
//! (`devices`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use rand::RngCore;

mod devices;

pub struct Secrets {
    dir: PathBuf,
    /// Phase 2: nothing an owner or bot token could be read from is written.
    enforce: bool,
    /// token -> bot_id, loaded at startup and kept in sync on issue/rotate.
    bot_tokens: Mutex<HashMap<String, String>>,
    /// sha256 of a device's token (hex) -> device_id.
    device_hashes: Mutex<HashMap<String, String>>,
    /// token -> peer_id for the `/peer` link. The same token serves both
    /// ends: the listening daemon accepts it, the dialing daemon presents it.
    peer_tokens: Mutex<HashMap<String, String>>,
    /// None in phase 2: the owner is known by their app or their OK (T4).
    client_token: Option<String>,
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn write_secret(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value)?;
    crate::permissions::private(path, false)?;
    Ok(())
}

fn read_token(path: &Path) -> anyhow::Result<Option<String>> {
    let token = fs::read_to_string(path)?.trim().to_string();
    Ok((!token.is_empty()).then_some(token))
}

fn remove_if_present(path: &Path) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

impl Secrets {
    /// Phase 1: every token file is read and written as before.
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        Self::open_for(dir, false)
    }

    /// `enforce` is phase 2 (`bot_bearer = "refuse"`): the client and bot
    /// token files are deleted and device tokens are reduced to hashes.
    pub fn open_for(dir: &Path, enforce: bool) -> anyhow::Result<Self> {
        fs::create_dir_all(dir)?;
        crate::permissions::private(dir, true)?;

        let client_path = dir.join("client.token");
        let client_token = if enforce {
            remove_if_present(&client_path).context("deleting the client token")?;
            None
        } else if client_path.exists() {
            Some(fs::read_to_string(&client_path)?.trim().to_string())
        } else {
            let t = random_token();
            write_secret(&client_path, &t).context("writing client token")?;
            Some(t)
        };

        let mut bot_tokens = HashMap::new();
        let mut peer_tokens = HashMap::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(bot_id) = name
                .strip_prefix("bot-")
                .and_then(|s| s.strip_suffix(".token"))
            {
                if enforce {
                    remove_if_present(&entry.path())?;
                } else if let Some(token) = read_token(&entry.path())? {
                    bot_tokens.insert(token, bot_id.to_string());
                }
            } else if let Some(peer_id) = name
                .strip_prefix("peer-")
                .and_then(|s| s.strip_suffix(".token"))
            {
                if let Some(token) = read_token(&entry.path())? {
                    peer_tokens.insert(token, peer_id.to_string());
                }
            }
        }
        let device_hashes = devices::load(dir, enforce)?;

        Ok(Self {
            dir: dir.to_path_buf(),
            enforce,
            bot_tokens: Mutex::new(bot_tokens),
            device_hashes: Mutex::new(device_hashes),
            peer_tokens: Mutex::new(peer_tokens),
            client_token,
        })
    }

    /// The client token, or "" in phase 2, where there is none.
    pub fn client_token(&self) -> &str {
        self.client_token.as_deref().unwrap_or_default()
    }

    pub fn verify_client(&self, token: &str) -> bool {
        self.client_token
            .as_deref()
            .is_some_and(|t| constant_time_eq(token.as_bytes(), t.as_bytes()))
    }

    /// Kept on disk in phase 1 only; phase 2 bots don't use one.
    fn keep_bot_token(&self, bot_id: &str, token: &str) -> anyhow::Result<()> {
        if self.enforce {
            return Ok(());
        }
        write_secret(&self.dir.join(format!("bot-{bot_id}.token")), token)
    }

    /// Existing or newly issued token for a bot.
    pub fn bot_token(&self, bot_id: &str) -> anyhow::Result<String> {
        let mut map = self.bot_tokens.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((token, _)) = map.iter().find(|(_, id)| id.as_str() == bot_id) {
            return Ok(token.clone());
        }
        let token = random_token();
        self.keep_bot_token(bot_id, &token)?;
        map.insert(token.clone(), bot_id.to_string());
        Ok(token)
    }

    /// bot_id for a presented bearer token, if valid.
    pub fn bot_for_token(&self, token: &str) -> Option<String> {
        let map = self.bot_tokens.lock().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .find(|(t, _)| constant_time_eq(t.as_bytes(), token.as_bytes()))
            .map(|(_, id)| id.clone())
    }

    /// Delete a bot's credential so an archived bot can never authenticate
    /// again. Without this the token stays valid in memory and on disk, and a
    /// deleted bot keeps full access to `/mcp` and `/hook`.
    pub fn remove_bot_token(&self, bot_id: &str) -> anyhow::Result<()> {
        let mut map = self.bot_tokens.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, id| id.as_str() != bot_id);
        remove_if_present(&self.dir.join(format!("bot-{bot_id}.token")))
    }

    pub fn rotate_bot_token(&self, bot_id: &str) -> anyhow::Result<String> {
        let mut map = self.bot_tokens.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, id| id.as_str() != bot_id);
        let token = random_token();
        self.keep_bot_token(bot_id, &token)?;
        map.insert(token.clone(), bot_id.to_string());
        Ok(token)
    }
}

impl Secrets {
    /// Issue a fresh link token for a peer that will dial this daemon.
    pub fn issue_peer_token(&self, peer_id: &str) -> anyhow::Result<String> {
        let token = random_token();
        self.store_peer_token(peer_id, &token)?;
        Ok(token)
    }

    /// Keep the token from a peer's invite, presented when dialing it.
    pub fn store_peer_token(&self, peer_id: &str, token: &str) -> anyhow::Result<()> {
        let mut map = self.peer_tokens.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, id| id.as_str() != peer_id);
        write_secret(&self.dir.join(format!("peer-{peer_id}.token")), token)?;
        map.insert(token.to_string(), peer_id.to_string());
        Ok(())
    }

    pub fn peer_for_token(&self, token: &str) -> Option<String> {
        let map = self.peer_tokens.lock().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .find(|(t, _)| constant_time_eq(t.as_bytes(), token.as_bytes()))
            .map(|(_, id)| id.clone())
    }

    pub fn peer_token(&self, peer_id: &str) -> Option<String> {
        let map = self.peer_tokens.lock().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .find(|(_, id)| id.as_str() == peer_id)
            .map(|(t, _)| t.clone())
    }

    /// Delete a revoked peer's token so neither end can use the link again.
    pub fn remove_peer_token(&self, peer_id: &str) -> anyhow::Result<()> {
        let mut map = self.peer_tokens.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, id| id.as_str() != peer_id);
        remove_if_present(&self.dir.join(format!("peer-{peer_id}.token")))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
