//! `hermesd hook <event> --endpoint <socket or pipe>` (H-044 §1): Claude
//! Code's lifecycle and permission hooks, sent over the local endpoint
//! instead of curl or PowerShell with a bearer token. The hook runs as a
//! child of the session, so the daemon knows whose it is from the process
//! tree, as it does for `bus-proxy`. It always exits 0: a hook failing must
//! never block the session, and printing nothing for a permission prompt
//! leaves it to the terminal, so a failure never allows anything.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::app::AppState;

/// A lifecycle event: `{event, body}`, answered with `{}`.
const HOOK: &str = "hermes/hook";
/// A permission prompt: `{body}`, answered with the hook output or null.
const PERMISSION: &str = "hermes/permission";

/// How long SessionStart keeps trying a daemon that isn't serving yet: it
/// is the only report of the session's inbox socket (H-038).
const SESSION_START_RETRY: Duration = Duration::from_secs(20);
/// How long a lifecycle event waits for the daemon, as curl's `-m 3` did.
const EVENT_TIMEOUT: Duration = Duration::from_secs(3);

/// Runs one hook. Returns the process exit code, always 0 once parsed.
pub async fn run(args: &[String]) -> i32 {
    let event = args.first().filter(|a| !a.starts_with("--"));
    let endpoint = args
        .iter()
        .position(|a| a == "--endpoint")
        .and_then(|i| args.get(i + 1));
    let (Some(event), Some(endpoint)) = (event, endpoint) else {
        eprintln!("hook: usage: hook <event> --endpoint <socket or pipe>");
        return 2;
    };
    let endpoint = full_endpoint(endpoint);
    let body = read_stdin().await;
    let (request, wait) = request(event, body, |key| std::env::var(key).ok());
    match send(&endpoint, &request, event == "SessionStart", wait).await {
        Ok(reply) => {
            let output = &reply["result"];
            if event == "PermissionRequest" && output.is_object() {
                println!("{output}");
            }
            if let Some(error) = reply.get("error") {
                eprintln!("hook {event}: {error}");
            }
        }
        Err(e) => eprintln!("hook {event}: {e:#}"),
    }
    0
}

/// On Windows the settings name the pipe without its `\\.\pipe\` prefix,
/// whose backslashes a shell would eat.
fn full_endpoint(endpoint: &str) -> String {
    if cfg!(windows) && !endpoint.contains(['\\', '/']) {
        format!(r"\\.\pipe\{endpoint}")
    } else {
        endpoint.to_string()
    }
}

/// Claude Code's hook payload, or `{}` when there is none to read.
async fn read_stdin() -> Value {
    let mut raw = Vec::new();
    let mut stdin = tokio::io::stdin();
    let read = stdin.read_to_end(&mut raw);
    match tokio::time::timeout(Duration::from_secs(5), read).await {
        Ok(Ok(_)) => serde_json::from_slice(&raw).unwrap_or_else(|_| json!({})),
        _ => json!({}),
    }
}

/// The JSON-RPC request for `event`, and how long to wait for its answer.
/// SessionStart adds the session's inbox socket and its token from the
/// environment Claude Code gives its hooks.
fn request(
    event: &str,
    mut body: Value,
    env: impl Fn(&str) -> Option<String>,
) -> (Value, Duration) {
    if !body.is_object() {
        body = json!({});
    }
    if event == "PermissionRequest" {
        let wait = Duration::from_secs(crate::approval::HOOK_TIMEOUT_SECS - 10);
        let request =
            json!({"jsonrpc": "2.0", "id": 1, "method": PERMISSION, "params": {"body": body}});
        return (request, wait);
    }
    if event == "SessionStart" {
        body["socket"] = json!(env("CLAUDE_CODE_MESSAGING_SOCKET").unwrap_or_default());
        body["msg_token"] = json!(env("CLAUDE_CODE_MESSAGING_TOKEN").unwrap_or_default());
    }
    let params = json!({"event": event, "body": body});
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": HOOK, "params": params});
    (request, EVENT_TIMEOUT)
}

/// Sends `request` and reads the one reply line. With `retry`, a daemon
/// that isn't listening yet (no socket, or refused) is tried again for
/// [`SESSION_START_RETRY`].
pub(crate) async fn send(
    endpoint: &str,
    request: &Value,
    retry: bool,
    wait: Duration,
) -> anyhow::Result<Value> {
    let deadline = tokio::time::Instant::now() + SESSION_START_RETRY;
    let stream = loop {
        match super::proxy::open(endpoint).await {
            Ok(stream) => break stream,
            Err(e) if retry && booting(&e) && tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => anyhow::bail!("can't reach the Hermes service at {endpoint}: {e}"),
        }
    };
    let exchange = async {
        let (reader, mut writer) = tokio::io::split(stream);
        writer.write_all(format!("{request}\n").as_bytes()).await?;
        writer.flush().await?;
        let line = BufReader::new(reader).lines().next_line().await?;
        let line = line.ok_or_else(|| anyhow::anyhow!("the daemon closed the connection"))?;
        Ok(serde_json::from_str(&line)?)
    };
    tokio::time::timeout(wait, exchange)
        .await
        .map_err(|_| anyhow::anyhow!("no answer from the daemon in {wait:?}"))?
}

/// The errors of a daemon that is starting: nothing listening yet.
fn booting(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

/// The endpoint's side: answers a hook request from `bot_id`'s session, or
/// `None` when `request` is no hook and goes to the MCP dispatcher.
pub(super) async fn serve(app: &Arc<AppState>, bot_id: &str, request: &Value) -> Option<Value> {
    let method = request.get("method").and_then(Value::as_str)?;
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or(json!({}));
    let body = params.get("body").cloned().unwrap_or(json!({}));
    let result = match method {
        HOOK => {
            let event = params
                .get("event")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !crate::mcp::on_hook(app, bot_id, event, &body) {
                return Some(super::ipc::error(id, -32602, "a hook needs an event"));
            }
            json!({})
        }
        PERMISSION => crate::approval::permission_output(app, bot_id, &body)
            .await
            .unwrap_or(Value::Null),
        _ => return None,
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_reports_the_inbox_socket_from_the_environment() {
        let env = |key: &str| Some(format!("<{key}>"));
        let (sent, wait) = request("SessionStart", json!({"source": "startup"}), env);
        assert_eq!(sent["method"], HOOK);
        let body = &sent["params"]["body"];
        assert_eq!(body["socket"], "<CLAUDE_CODE_MESSAGING_SOCKET>");
        assert_eq!(body["msg_token"], "<CLAUDE_CODE_MESSAGING_TOKEN>");
        assert_eq!(body["source"], "startup");
        assert_eq!(wait, EVENT_TIMEOUT);
    }

    #[test]
    fn other_events_forward_the_payload_untouched() {
        let payload = json!({"message": "Claude needs your permission", "transcript_path": "/t"});
        let (sent, _) = request("Notification", payload.clone(), |_| None);
        assert_eq!(sent["params"]["event"], "Notification");
        assert_eq!(sent["params"]["body"], payload);
        let (sent, wait) = request("Stop", json!("not an object"), |_| None);
        assert_eq!(sent["params"]["body"], json!({}));
        assert_eq!(wait, EVENT_TIMEOUT);
    }

    #[test]
    fn a_permission_prompt_waits_for_the_owner() {
        let prompt = json!({"tool_name": "Bash", "tool_input": {"command": "ls"}});
        let (sent, wait) = request("PermissionRequest", prompt.clone(), |_| None);
        assert_eq!(sent["method"], PERMISSION);
        assert_eq!(sent["params"]["body"], prompt);
        assert_eq!(wait.as_secs(), crate::approval::HOOK_TIMEOUT_SECS - 10);
    }

    #[test]
    fn a_daemon_still_starting_is_retried() {
        use std::io::{Error, ErrorKind};
        assert!(booting(&Error::from(ErrorKind::NotFound)));
        assert!(booting(&Error::from(ErrorKind::ConnectionRefused)));
        assert!(!booting(&Error::from(ErrorKind::PermissionDenied)));
    }
}
