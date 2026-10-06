//! `hermesd release publish …`: DevOps publishes a build from its terminal.
//! The command calls `release_publish` on the running daemon with the bot's
//! own token, so the daemon checks the role and the Publish extra exactly as
//! it does over MCP, and does the copying itself.

use serde_json::{json, Value};

use crate::config::Config;

const USAGE: &str = "usage:
  hermesd release publish <release> <file> [--platform <p>] [--version <v>] [--bundle-id <id>]
  hermesd release install <release> [--dry-run]";

const FLAGS: [&str; 3] = ["--platform", "--version", "--bundle-id"];

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("publish") => {}
        // A tester installing a release on their computer (B8).
        Some("install") => return super::install::run(cfg, &args[1..]).await,
        // The install job's app swap, never a bot's session (H-117 X1).
        Some("apply-app") => return super::install::apply::run(cfg, &args[1..]),
        _ => anyhow::bail!("{USAGE}"),
    }
    let (positional, flags) = split(&args[1..])?;
    let [release, file] = positional.as_slice() else {
        anyhow::bail!("{USAGE}");
    };
    let token = crate::brand::env_var_os("TOKEN")
        .and_then(|t| t.into_string().ok())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "run this from DevOps's session: {} is not set",
                crate::brand::BOT_TOKEN_ENV
            )
        })?;
    let file = std::path::absolute(file)?;
    let mut arguments = json!({ "release_id": release, "file": file.display().to_string() });
    for (flag, value) in flags {
        let key = flag.trim_start_matches("--").replace('-', "_");
        arguments[key] = json!(value);
    }
    let port = crate::home::runtime_port(&cfg.home).unwrap_or(cfg.port);
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "release_publish", "arguments": arguments }
    });
    let reply = post(port, &token, &body).await?;
    println!("{}", outcome(&reply)?);
    Ok(())
}

/// One JSON-RPC request to the daemon's `/mcp` on loopback, over plain
/// HTTP/1.1: the only client the command needs.
async fn post(port: u16, token: &str, body: &Value) -> anyhow::Result<Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = body.to_string();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| anyhow::anyhow!("hermesd is not answering on port {port}: {e}"))?;
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    let raw = String::from_utf8_lossy(&raw);
    let (head, payload) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("hermesd sent no answer"))?;
    let status = head.split_whitespace().nth(1).unwrap_or_default();
    if status == "401" {
        anyhow::bail!("hermesd doesn't know this bot token");
    }
    anyhow::ensure!(status == "200", "hermesd answered {status}");
    Ok(serde_json::from_str(payload)?)
}

/// A known option and its value.
type Flag<'a> = (&'a str, &'a str);

/// Positional arguments, and the known flags with their values.
fn split(args: &[String]) -> anyhow::Result<(Vec<&str>, Vec<Flag<'_>>)> {
    let mut positional = Vec::new();
    let mut flags = Vec::new();
    let mut it = args.iter().map(String::as_str);
    while let Some(arg) = it.next() {
        if let Some(flag) = FLAGS.iter().find(|f| **f == arg) {
            let value = it
                .next()
                .ok_or_else(|| anyhow::anyhow!("{flag} needs a value\n{USAGE}"))?;
            flags.push((*flag, value));
        } else if arg.starts_with("--") {
            anyhow::bail!("unknown option {arg}\n{USAGE}");
        } else {
            positional.push(arg);
        }
    }
    Ok((positional, flags))
}

/// What to print for the daemon's answer, or its refusal as the error.
fn outcome(reply: &Value) -> anyhow::Result<String> {
    if let Some(error) = reply.get("error") {
        anyhow::bail!("{}", error["message"].as_str().unwrap_or("request refused"));
    }
    let result = &reply["result"];
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    if result["isError"] == json!(true) {
        anyhow::bail!("refused: {text}");
    }
    let payload: Value = serde_json::from_str(text)?;
    let p = &payload["published"];
    let verb = if p["already_published"] == json!(true) {
        "already published"
    } else {
        "published"
    };
    Ok(format!(
        "{verb}: {}\ninstall: {}\nsha256: {}",
        p["url"].as_str().unwrap_or_default(),
        p["install_url"].as_str().unwrap_or_default(),
        p["sha256"].as_str().unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_string()).collect()
    }

    #[test]
    fn flags_and_positionals_split() {
        let args = strings(&["r1", "--version", "0.16.0", "/b/app.zip"]);
        let (pos, flags) = split(&args).unwrap();
        assert_eq!(pos, ["r1", "/b/app.zip"]);
        assert_eq!(flags, [("--version", "0.16.0")]);
        assert!(split(&strings(&["r1", "--force"])).is_err());
        assert!(split(&strings(&["r1", "--platform"])).is_err());
    }

    #[test]
    fn a_refusal_is_an_error() {
        let refused = json!({"result": {"isError": true, "content": [{"text": "no role"}]}});
        assert!(outcome(&refused)
            .unwrap_err()
            .to_string()
            .contains("no role"));
        let payload = json!({"published": {"url": "https://h/r/ios/a.ipa", "sha256": "ab",
            "install_url": "itms-services://x", "already_published": false}});
        let ok = json!({"result": {"content": [{"text": payload.to_string()}]}});
        let text = outcome(&ok).unwrap();
        assert!(
            text.starts_with("published: https://h/r/ios/a.ipa"),
            "{text}"
        );
    }
}
