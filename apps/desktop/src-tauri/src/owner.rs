//! A one-time owner ticket from the daemon's local endpoint (H-044 T4). The
//! daemon grants it because this process is the app it pinned at `service
//! install`; nothing secret is stored or passed in the environment.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
#[cfg(unix)]
use std::time::Duration;

/// `None` when there is no endpoint (an older daemon) or it refused: the
/// caller falls back to `client.token`.
pub fn ticket(home: &Path) -> Option<String> {
    let endpoint = std::fs::read_to_string(home.join("run").join("endpoint")).ok()?;
    let endpoint = endpoint.trim();
    #[cfg(unix)]
    let stream = {
        let stream = std::os::unix::net::UnixStream::connect(endpoint).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        stream
    };
    #[cfg(windows)]
    let stream = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(endpoint)
        .ok()?;
    let mut writer = stream.try_clone().ok()?;
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"hermes/owner_ticket","params":{}}"#;
    writer.write_all(format!("{request}\n").as_bytes()).ok()?;
    writer.flush().ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let reply: serde_json::Value = serde_json::from_str(&line).ok()?;
    reply["result"]["ticket"].as_str().map(str::to_string)
}
