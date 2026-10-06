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

async fn connect(endpoint: &str) -> anyhow::Result<()> {
    let stream = open(endpoint)
        .await
        .map_err(|e| anyhow::anyhow!("can't reach the Hermes service at {endpoint}: {e}"))?;
    relay(stream).await
}

#[cfg(unix)]
pub(super) type Stream = tokio::net::UnixStream;
#[cfg(windows)]
pub(super) type Stream = tokio::net::windows::named_pipe::NamedPipeClient;

/// Connects to the daemon's endpoint. Shared with `hermesd hook`.
#[cfg(unix)]
pub(super) async fn open(endpoint: &str) -> std::io::Result<Stream> {
    tokio::net::UnixStream::connect(endpoint).await
}

#[cfg(windows)]
pub(super) async fn open(endpoint: &str) -> std::io::Result<Stream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    // ERROR_PIPE_BUSY: every instance is taken for a moment; try again.
    const PIPE_BUSY: i32 = 231;
    // ERROR_FILE_NOT_FOUND: between taking a client and creating the next
    // instance, the pipe briefly has none, so its name doesn't exist (H-120).
    const NOT_FOUND: i32 = 2;
    let mut tries = 0;
    loop {
        match ClientOptions::new().open(endpoint) {
            Ok(pipe) => return Ok(pipe),
            Err(e) if matches!(e.raw_os_error(), Some(PIPE_BUSY | NOT_FOUND)) && tries < 50 => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(e) => return Err(e),
        }
    }
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
