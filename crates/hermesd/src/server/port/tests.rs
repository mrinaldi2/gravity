use super::*;

const LOOPBACK: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

#[test]
fn recognizes_windows_reserved_ports_without_masking_other_errors() {
    assert_eq!(
        port_unavailable(&std::io::Error::from_raw_os_error(10013)),
        cfg!(windows)
    );
    assert!(!port_unavailable(&std::io::Error::from(
        std::io::ErrorKind::InvalidInput
    )));
}

#[tokio::test]
async fn keeps_the_configured_port_when_it_is_available() {
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let port = reserved.local_addr().expect("reserved address").port();
    drop(reserved);

    let bound = bind(&[LOOPBACK], port, PortPolicy::Negotiate)
        .await
        .expect("bind configured port");

    assert_eq!(bound.port(), port);
    assert!(!bound.negotiated());
}

#[tokio::test]
async fn negotiates_when_the_configured_port_is_taken() {
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let configured = reserved.local_addr().expect("reserved address").port();

    let bound = bind_with_grace(
        &[LOOPBACK],
        configured,
        PortPolicy::Negotiate,
        Duration::ZERO,
    )
    .await
    .expect("bind fallback port");

    assert_ne!(bound.port(), configured);
    assert!(bound.negotiated());
}

#[tokio::test]
async fn waits_out_a_port_that_clears_during_the_grace_period() {
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let configured = reserved.local_addr().expect("reserved address").port();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(reserved);
    });

    let bound = bind_with_grace(
        &[LOOPBACK],
        configured,
        PortPolicy::Negotiate,
        Duration::from_secs(5),
    )
    .await
    .expect("bind configured port");

    assert_eq!(bound.port(), configured);
    assert!(!bound.negotiated());
}

#[tokio::test]
async fn the_grace_period_never_delays_a_strict_launch() {
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let configured = reserved.local_addr().expect("reserved address").port();

    let started = std::time::Instant::now();
    let error = match bind(&[LOOPBACK], configured, PortPolicy::Strict).await {
        Ok(_) => panic!("a strict launch must fail on a collision"),
        Err(error) => error,
    };

    assert!(error.to_string().contains(&configured.to_string()));
    assert!(started.elapsed() < PORT_GRACE);
}

#[tokio::test]
async fn a_freed_configured_port_wakes_the_reclaim_watch() {
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let configured = reserved.local_addr().expect("reserved address").port();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(reserved);
    });

    let waited = tokio::time::timeout(
        Duration::from_secs(5),
        wait_for_configured_port(vec![LOOPBACK], configured, Duration::from_millis(50)),
    )
    .await;

    assert!(waited.is_ok(), "the watch should end once the port is free");
}

/// A daemon on 0.8.0 holding the port, answering `/health` as a real one
/// does: it reads each request before answering and keeps listening. Its
/// first `ignored` probes go unanswered, as from a daemon too busy to reply.
/// (H-183: an occupant that answered once without reading the request was
/// reset on Windows, which drops the unread reply, and then left the port.)
fn occupant(reserved: tokio::net::TcpListener, ignored: usize) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tokio::spawn(async move {
        let mut unanswered = Vec::new();
        loop {
            let (mut stream, _) = reserved.accept().await.expect("occupant accept");
            if unanswered.len() < ignored {
                unanswered.push(stream);
                continue;
            }
            let mut request = Vec::new();
            let mut chunk = [0_u8; 256];
            while !request.ends_with(b"\r\n\r\n") {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&chunk[..n]),
                }
            }
            let body = "{\"status\":\"ok\",\"version\":\"0.8.0\"}";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
}

#[tokio::test]
async fn refuses_to_move_aside_for_another_daemon() {
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let configured = reserved.local_addr().expect("reserved address").port();
    occupant(reserved, 0);

    let error = bind(&[LOOPBACK], configured, PortPolicy::Negotiate)
        .await
        .err()
        .expect("a live daemon must stop the launch");

    assert!(error.to_string().contains("0.8.0"), "{error}");
}

/// A daemon that misses the first probe (a loaded machine) is asked again
/// while the port is waited out, and still stops the launch.
#[tokio::test]
async fn a_daemon_too_busy_for_the_first_probe_still_stops_the_launch() {
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let configured = reserved.local_addr().expect("reserved address").port();
    occupant(reserved, 1);

    let error = bind(&[LOOPBACK], configured, PortPolicy::Negotiate)
        .await
        .err()
        .expect("a live daemon must stop the launch");

    assert!(error.to_string().contains("0.8.0"), "{error}");
}

#[tokio::test]
async fn rejects_a_collision_without_negotiation() {
    let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let configured = reserved.local_addr().expect("reserved address").port();

    let error = bind(&[LOOPBACK], configured, PortPolicy::Strict)
        .await
        .err()
        .expect("collision must fail");

    assert!(
        error.to_string().contains(&configured.to_string()),
        "{error}"
    );
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::AddrInUse)
    );
}

fn config_with(bind: Vec<IpAddr>, negotiate_port: bool) -> crate::config::Config {
    crate::config::Config {
        bind,
        negotiate_port,
        ..crate::config::Config::default()
    }
}

#[test]
fn only_a_loopback_daemon_asked_to_negotiate_may_negotiate() {
    let loopback = config_with(vec![LOOPBACK], true);
    assert_eq!(port_policy(&loopback, true), PortPolicy::Negotiate);
    assert_eq!(port_policy(&loopback, false), PortPolicy::Strict);

    let opted_out = config_with(vec![LOOPBACK], false);
    assert_eq!(port_policy(&opted_out, true), PortPolicy::Strict);

    let reachable = config_with(vec![LOOPBACK, IpAddr::from([100, 64, 0, 1])], true);
    assert_eq!(port_policy(&reachable, true), PortPolicy::Strict);
}

#[test]
fn reads_the_version_out_of_a_health_response() {
    let health_version = |r: &str| health_field(r, "version");
    let ok = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n\
              {\"status\":\"ok\",\"version\":\"0.9.0\",\"binary_sha256\":\"abc\"}";
    assert_eq!(health_version(ok).as_deref(), Some("0.9.0"));
    assert_eq!(health_field(ok, "binary_sha256").as_deref(), Some("abc"));

    assert_eq!(health_version("HTTP/1.1 404 Not Found\r\n\r\n"), None);
    assert_eq!(health_version("SSH-2.0-OpenSSH_9.0\r\n"), None);
    assert_eq!(
        health_version("HTTP/1.1 200 OK\r\n\r\n{\"status\":\"ok\"}"),
        None
    );
}
