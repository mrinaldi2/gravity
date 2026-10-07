//! What the daemon logs while a test runs on this thread, for tests that
//! check a log line says why (H-209).
//!
//! One global subscriber for the whole test binary, installed once, writes
//! each line to the buffer of the test capturing on that thread, if any. A
//! scoped subscriber per test isn't deterministic: while they come and go on
//! other threads, tracing's shared callsite interest and max level can
//! leave a callsite off for this one, and a line goes missing (seen on
//! Windows, 0.17.4).

use std::cell::RefCell;
use std::sync::{Arc, Mutex, Once};

thread_local! {
    /// The buffer of the test capturing on this thread.
    static CAPTURING: RefCell<Option<Logs>> = const { RefCell::new(None) };
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<u8>>>);

/// Writes to the capturing test's buffer; drops the line when none is.
struct ThisThread;

impl std::io::Write for ThisThread {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        CAPTURING.with(|c| {
            if let Some(logs) = c.borrow().as_ref() {
                logs.0.lock().unwrap().extend_from_slice(buf);
            }
        });
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Stops the capture when the test is done with it.
pub(super) struct Capture;

impl Drop for Capture {
    fn drop(&mut self) {
        CAPTURING.with(|c| *c.borrow_mut() = None);
    }
}

impl Logs {
    pub(super) fn capture(&self) -> Capture {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let subscriber = tracing_subscriber::fmt()
                .with_writer(|| ThisThread)
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("the test binary's one log subscriber");
        });
        CAPTURING.with(|c| *c.borrow_mut() = Some(self.clone()));
        Capture
    }

    pub(super) fn lines(&self, needle: &str) -> Vec<String> {
        String::from_utf8_lossy(&self.0.lock().unwrap())
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    }
}
