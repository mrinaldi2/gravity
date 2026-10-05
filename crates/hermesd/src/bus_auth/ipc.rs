//! The bus's local endpoint (H-044 §1): a Unix socket under the daemon's
//! home, or a named pipe only this user may open. Its path is no secret;
//! what a connection may do is decided by which process is on the other end.
//! Each line in is a JSON-RPC request, served by the same dispatcher as
//! `POST /mcp`, as the bot whose session the caller runs in.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use super::os::OsProcessTable;
use super::session::{session_of, ProcessTable};
use crate::app::AppState;
use crate::config::Config;

/// JSON-RPC error for a caller in no bot session, or whose session ended.
const NOT_A_SESSION: i64 = -32001;

/// Where the daemon listens and `bus-proxy` connects.
#[cfg(unix)]
pub fn endpoint(cfg: &Config) -> PathBuf {
    cfg.home.join("run").join("bus.sock")
}

#[cfg(windows)]
pub fn endpoint(cfg: &Config) -> PathBuf {
    PathBuf::from(windows::pipe_name(&cfg.home))
}

/// `bus-proxy`'s arguments for this daemon's endpoint.
pub fn proxy_args(cfg: &Config) -> Vec<String> {
    vec![
        "bus-proxy".to_string(),
        "--endpoint".to_string(),
        endpoint(cfg).to_string_lossy().into_owned(),
    ]
}

/// Serves the endpoint until the daemon stops.
#[cfg(unix)]
pub async fn serve(app: Arc<AppState>) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let path = endpoint(&app.cfg);
    let dir = path.parent().expect("the socket has a directory");
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    // A socket left by a daemon that died; one daemon serves a home.
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!(path = %path.display(), "bus endpoint listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let pid = stream
            .peer_cred()
            .ok()
            .and_then(|cred| cred.pid())
            .and_then(|pid| u32::try_from(pid).ok());
        tokio::spawn(serve_connection(app.clone(), stream, pid));
    }
}

#[cfg(windows)]
pub async fn serve(app: Arc<AppState>) -> anyhow::Result<()> {
    windows::serve(app).await
}

/// One connection: resolved once to the bot whose session the caller runs
/// in, then checked again before every request, so a restarted session's
/// old processes are cut off.
pub(super) async fn serve_connection<S>(app: Arc<AppState>, stream: S, peer_pid: Option<u32>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let roots = app.supervisor.session_roots();
    let table = OsProcessTable;
    let resolved = peer_pid
        .and_then(|pid| table.info(pid))
        .and_then(|peer| session_of(&roots, &table, peer));
    if resolved.is_none() {
        tracing::warn!(?peer_pid, "bus connection from no bot session refused");
    }
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            if send(&mut writer, &error(Value::Null, -32700, "parse error"))
                .await
                .is_err()
            {
                return;
            }
            continue;
        };
        let current = resolved
            .as_ref()
            .filter(|(root, bot)| roots.bot_of(*root).as_deref() == Some(bot.as_str()));
        let Some((_, bot_id)) = current else {
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let _ = send(&mut writer, &error(id, NOT_A_SESSION, "not a bot session")).await;
            return;
        };
        if let Some(reply) = crate::mcp::rpc_response(&app, bot_id, &request).await {
            if send(&mut writer, &reply).await.is_err() {
                return;
            }
        }
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn send<W: AsyncWrite + Unpin>(writer: &mut W, reply: &Value) -> std::io::Result<()> {
    let mut line = reply.to_string();
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await
}

#[cfg(windows)]
mod windows;
