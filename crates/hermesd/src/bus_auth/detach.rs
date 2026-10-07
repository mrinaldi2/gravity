//! Bot-side processes detached from the bot's terminal (H-195 D2b, CE-029
//! M1): a process whose controlling terminal is the bot's PTY can push
//! keystrokes into its composer with TIOCSTI. One in a session of its own
//! gets `EPERM` instead.

/// `hermesd mcp-exec -- <command> [args…]`: runs a stdio MCP server in a new
/// session. Returns an exit code only when it could not be started.
pub fn mcp_exec(args: &[String]) -> i32 {
    let args = match args.first().map(String::as_str) {
        Some("--") => &args[1..],
        _ => args,
    };
    let Some((command, rest)) = args.split_first() else {
        eprintln!("mcp-exec: usage: mcp-exec -- <command> [args…]");
        return 2;
    };
    setsid();
    let mut cmd = std::process::Command::new(command);
    cmd.args(rest);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = cmd.exec();
        eprintln!("mcp-exec: {command}: {error}");
        127
    }
    #[cfg(not(unix))]
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("mcp-exec: {command}: {error}");
            127
        }
    }
}

/// Leaves the bot's terminal session. Harmless when already detached: a
/// group leader's `EPERM` is ignored.
pub fn setsid() {
    #[cfg(unix)]
    // SAFETY: setsid takes no arguments and only changes this process's
    // session; its one failure (already a group leader) is ignored.
    unsafe {
        libc::setsid();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::config::Config;

    #[test]
    fn mcp_servers_are_detached_only_under_the_composer_switch() {
        let entry = json!({"type": "stdio", "command": "npx", "args": ["-y", "pkg"]});
        let mut cfg = Config::default();
        assert_eq!(crate::bus_auth::detached(&cfg, entry.clone()), entry);
        cfg.delivery.composer = true;
        let wrapped = crate::bus_auth::detached(&cfg, entry);
        if cfg!(unix) {
            assert_eq!(
                wrapped["args"],
                json!(["mcp-exec", "--", "npx", "-y", "pkg"])
            );
            assert_eq!(wrapped["type"], "stdio");
        }
    }
}
