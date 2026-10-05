//! `hermesd quiesce start|status|resume <release>` (H-117 Q4): the
//! `install_quiesce` tool, asked over the local endpoint, which knows the
//! bot by its process (H-044). `hermesd release install` calls the same.

use std::time::Duration;

use serde_json::{json, Value};

use crate::config::Config;

const USAGE: &str = "usage: hermesd quiesce <start|status|extend|resume> <release> [--version <v>]";

/// A pause stops sessions and reaps for a few seconds before answering.
const WAIT: Duration = Duration::from_secs(180);

/// Asks the daemon, as this session's bot, to `action` the pause.
pub async fn call(
    cfg: &Config,
    action: &str,
    release: &str,
    version: Option<&str>,
) -> anyhow::Result<Value> {
    let mut arguments = json!({ "action": action, "release_id": release });
    if let Some(version) = version {
        arguments["version"] = json!(version);
    }
    call_with(cfg, arguments).await
}

/// After each install step: the pause's deadline moves forward, so the
/// dead-man switch can't resume projects mid-install (ARCH-R50 S2). Only
/// warns when it can't.
pub async fn extend(cfg: &Config, release: &str) {
    if let Err(e) = call(cfg, "extend", release, None).await {
        eprintln!("warning: couldn't extend the pause's deadline: {e:#}");
    }
}

/// Right before the handoff: the pause is in `install_started`, with the
/// hash of the daemon binary being installed when there is one (ARCH-R50 S1).
pub async fn install_started(
    cfg: &Config,
    release: &str,
    binary: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let mut arguments = json!({ "action": "install_started", "release_id": release });
    if let Some(binary) = binary {
        arguments["binary_sha256"] = json!(super::outcome::file_sha256(binary)?);
    }
    call_with(cfg, arguments).await.map(drop)
}

async fn call_with(cfg: &Config, arguments: Value) -> anyhow::Result<Value> {
    let endpoint = crate::bus_auth::ipc::endpoint(cfg).display().to_string();
    let request = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "install_quiesce", "arguments": arguments },
    });
    let reply = crate::bus_auth::hook::send(&endpoint, &request, false, WAIT).await?;
    if let Some(error) = reply.get("error") {
        anyhow::bail!(
            "{} (run this from the installing bot's own session)",
            error["message"].as_str().unwrap_or("refused")
        );
    }
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    anyhow::ensure!(reply["result"]["isError"] != json!(true), "refused: {text}");
    Ok(serde_json::from_str(text)?)
}

/// What the owner and the bot read of a pause's report.
pub fn describe(answer: &Value) -> String {
    let report = &answer["report"];
    let mut out = super::outcome::summary(report);
    for holder in report["unresolved"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "\n  still holding the home: pid {} {} ({})",
            holder["pid"],
            holder["command"].as_str().unwrap_or("?"),
            holder["path"].as_str().unwrap_or("?"),
        ));
    }
    if report["services_changed"] == true {
        out.push_str(
            "\n  note: the list of background services changed after the Hermes service \
             started; this install used the earlier list",
        );
    }
    out
}

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let action = args.first().map(String::as_str);
    let release = args.get(1).map(String::as_str);
    let version = args
        .iter()
        .position(|a| a == "--version")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    let (Some(action @ ("start" | "status" | "extend" | "resume")), Some(release)) =
        (action, release)
    else {
        anyhow::bail!("{USAGE}");
    };
    let answer = call(cfg, action, release, version).await?;
    match action {
        "start" => {
            println!("{}", describe(&answer));
            anyhow::ensure!(
                answer["proceed"] == true,
                "the install must wait until the processes above let go of the home"
            );
        }
        _ => println!("{}", serde_json::to_string_pretty(&answer)?),
    }
    Ok(())
}
