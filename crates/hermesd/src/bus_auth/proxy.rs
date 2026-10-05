//! `hermesd bus-proxy --endpoint <socket or pipe>` (H-044 §1): the stdio MCP
//! server a session starts as its own child. It relays JSON-RPC lines
//! between its stdin/stdout and the daemon's local endpoint, and holds
//! nothing secret: the daemon knows who it is from its place in the process
//! tree. It prints nothing of its own on stdout, which belongs to MCP.

use tokio::io::{AsyncRead, AsyncWrite};

/// Relays until either side closes. Returns the process exit code.
pub async fn run(args: &[String]) -> i32 {
    let Some(endpoint) = args
        .iter()
        .position(|a| a == "--endpoint")
        .and_then(|i| args.get(i + 1))
    else {
        eprintln!("bus-proxy: --endpoint <socket or pipe> is required");
        return 2;
    };
    match connect(endpoint).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("bus-proxy: {e:#}");
            1
        }
    }
}

#[cfg(unix)]
async fn connect(endpoint: &str) -> anyhow::Result<()> {
    let stream = tokio::net::UnixStream::connect(endpoint)
        .await
        .map_err(|e| anyhow::anyhow!("can't reach the Hermes service at {endpoint}: {e}"))?;
    relay(stream).await
}

#[cfg(windows)]
async fn connect(endpoint: &str) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::ClientOptions;
    // ERROR_PIPE_BUSY: every instance is taken for a moment; try again.
    const PIPE_BUSY: i32 = 231;
    let mut tries = 0;
    let pipe = loop {
        match ClientOptions::new().open(endpoint) {
            Ok(pipe) => break pipe,
            Err(e) if e.raw_os_error() == Some(PIPE_BUSY) && tries < 50 => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(e) => anyhow::bail!("can't reach the Hermes service at {endpoint}: {e}"),
        }
    };
    relay(pipe).await
}

/// Copies stdin to the endpoint and the endpoint to stdout. The daemon
/// closing the connection (a refused or ended session) ends the proxy.
async fn relay<S: AsyncRead + AsyncWrite>(stream: S) -> anyhow::Result<()> {
    let (mut from_daemon, mut to_daemon) = tokio::io::split(stream);
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    tokio::select! {
        sent = tokio::io::copy(&mut stdin, &mut to_daemon) => { sent?; }
        received = tokio::io::copy(&mut from_daemon, &mut stdout) => { received?; }
    }
    Ok(())
}
