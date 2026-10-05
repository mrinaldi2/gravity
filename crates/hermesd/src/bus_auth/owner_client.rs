//! Asking the daemon for an owner ticket over its local endpoint (H-044 T4):
//! what a CLI owner command does instead of reading `client.token`.

use std::path::Path;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// What asking for a ticket came to.
pub enum Asked {
    Ticket(String),
    /// The daemon refused, with its reason (not the app; the owner said no).
    Refused(String),
    /// No endpoint to ask: a daemon from before H-044.
    NoEndpoint,
}

/// Sends `method` to the endpoint the daemon at `home` announced and waits
/// for the answer, which for a card can take as long as the owner does.
pub async fn ask(home: &Path, method: &str, params: Value) -> anyhow::Result<Asked> {
    let Ok(endpoint) = std::fs::read_to_string(super::ipc::endpoint_file(home)) else {
        return Ok(Asked::NoEndpoint);
    };
    let endpoint = endpoint.trim();
    #[cfg(unix)]
    let stream = match tokio::net::UnixStream::connect(endpoint).await {
        Ok(stream) => stream,
        Err(_) => return Ok(Asked::NoEndpoint),
    };
    #[cfg(windows)]
    let stream = match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
        Ok(pipe) => pipe,
        Err(_) => return Ok(Asked::NoEndpoint),
    };
    exchange(stream, method, params).await
}

async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    method: &str,
    params: Value,
) -> anyhow::Result<Asked> {
    let (reader, mut writer) = tokio::io::split(stream);
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    writer.write_all(format!("{request}\n").as_bytes()).await?;
    writer.flush().await?;
    let Some(line) = BufReader::new(reader).lines().next_line().await? else {
        anyhow::bail!("the Hermes service closed the connection without answering");
    };
    let reply: Value = serde_json::from_str(&line)?;
    if let Some(ticket) = reply["result"]["ticket"].as_str() {
        return Ok(Asked::Ticket(ticket.to_string()));
    }
    let reason = reply["error"]["message"].as_str().unwrap_or("refused");
    Ok(Asked::Refused(reason.to_string()))
}

/// The owner credential for a CLI command: a ticket the owner allowed on a
/// card, or `client.token` from a daemon too old to ask.
pub async fn cli_credential(home: &Path, secrets: &Path) -> anyhow::Result<String> {
    let command = std::env::args()
        .map(|a| a.rsplit(['/', '\\']).next().unwrap_or(&a).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("Asking for your OK in The Hermes app…");
    match ask(home, "hermes/owner_request", json!({ "command": command })).await? {
        Asked::Ticket(ticket) => Ok(ticket),
        Asked::Refused(reason) => anyhow::bail!("not allowed: {reason}"),
        Asked::NoEndpoint => {
            let token = std::fs::read_to_string(secrets.join("client.token"))
                .map_err(|e| anyhow::anyhow!("cannot read the owner token: {e}"))?;
            Ok(token.trim().to_string())
        }
    }
}
