//! `hermesd board import <path> [--project <name>] [--dry-run]`: the owner's
//! import from a terminal. The file is read here and sent to the running
//! daemon over its control plane with the owner token, so the daemon that
//! writes is the board's home and no other.

use serde_json::json;

use crate::config::Config;
use crate::peer::cli::Daemon;

const USAGE: &str = "usage: hermesd board import <path> [--project <name>] [--dry-run]";

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let project = flag("--project");
    let mut positional = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--project" | "--config" => {
                rest.next();
            }
            "--dry-run" => {}
            _ => positional.push(arg.as_str()),
        }
    }
    let ["import", path] = positional.as_slice() else {
        anyhow::bail!("{USAGE}");
    };
    let markdown =
        std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("can't read {path}: {e}"))?;
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let mut daemon = Daemon::connect(cfg).await?;
    let mut req = json!({ "type": "board_import", "markdown": markdown, "dry_run": dry_run });
    if let Some(project) = project {
        req["project"] = json!(project);
    }
    let reply = daemon.request(req).await?;
    print!("{}", reply["summary"].as_str().unwrap_or_default());
    if dry_run {
        println!("Dry run: nothing was written.");
    }
    Ok(())
}
