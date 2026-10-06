//! Panic containment for request handlers (H-167, H-170). A handler that
//! panics answers its request `internal` and leaves the connection it came
//! on serving. In 0.17.0 one panic ended the owner's connection task with
//! the socket still open, and the app waited forever on every request after
//! it.
//!
//! What a handler leaves half done does not outlive the unwind: database
//! writes run in `rusqlite` transactions, which roll back when dropped, and
//! every shared lock is taken with `unwrap_or_else(|e| e.into_inner())`, so
//! a lock a panic poisoned still opens for the next request.

use std::any::Any;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};

use futures::FutureExt;

/// What a panic carried, for the log.
pub fn panic_text(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-text panic".to_string())
}

/// The message a request that panicked is answered with.
pub fn internal_message(kind: &str) -> String {
    format!("the service failed on '{kind}'; it's in the service log")
}

/// Runs `work`; if it panics, logs the panic under the request's `kind` and
/// returns `on_panic()` instead.
pub fn run<T>(kind: &str, work: impl FnOnce() -> T, on_panic: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(value) => value,
        Err(panic) => {
            logged(kind, panic.as_ref());
            on_panic()
        }
    }
}

/// [`run`] for a handler's future: one answered off the connection's task.
pub async fn run_async<T>(
    kind: &str,
    work: impl Future<Output = T>,
    on_panic: impl FnOnce() -> T,
) -> T {
    match AssertUnwindSafe(work).catch_unwind().await {
        Ok(value) => value,
        Err(panic) => {
            logged(kind, panic.as_ref());
            on_panic()
        }
    }
}

fn logged(kind: &str, panic: &(dyn Any + Send)) {
    tracing::error!(
        kind,
        panic = %panic_text(panic),
        "a request's handler panicked; it answers `internal` and the connection goes on"
    );
}

/// A daemon on a fresh temporary home, for the containment tests.
#[cfg(test)]
pub(crate) mod testing {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use crate::app::AppState;
    use crate::config::{Config, RuntimeKind};
    use crate::db::Db;

    pub(crate) struct Daemon {
        pub app: Arc<AppState>,
        pub addr: SocketAddr,
        _home: tempfile::TempDir,
    }

    pub(crate) async fn daemon() -> Daemon {
        let home = tempfile::tempdir().expect("tempdir");
        let mut cfg = Config {
            home: home.path().to_path_buf(),
            runtime: RuntimeKind::Double,
            ..Config::default()
        };
        let db = Db::open(&cfg.db_path()).expect("db");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        cfg.port = addr.port();
        let app = AppState::new(cfg, db).expect("app");
        let router = crate::server::router(app.clone());
        tokio::spawn(async move {
            let service = router.into_make_service_with_connect_info::<SocketAddr>();
            let _ = axum::serve(listener, service).await;
        });
        Daemon {
            app,
            addr,
            _home: home,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_the_fallback() {
        assert_eq!(run("t", || panic!("boom"), || 7), 7);
        assert_eq!(run("t", || 1, || 7), 1);
    }

    #[tokio::test]
    async fn a_future_that_panics_becomes_the_fallback() {
        let answer = run_async("t", async { panic!("boom") }, || "internal").await;
        assert_eq!(answer, "internal");
    }

    #[test]
    fn the_panic_text_is_kept() {
        let text = catch_unwind(|| panic!("at {}", 3)).unwrap_err();
        assert_eq!(panic_text(text.as_ref()), "at 3");
        let text = catch_unwind(|| panic!("plain")).unwrap_err();
        assert_eq!(panic_text(text.as_ref()), "plain");
    }
}
