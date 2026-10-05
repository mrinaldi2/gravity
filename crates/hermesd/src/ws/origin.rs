//! Which WebSocket origins may connect.

use std::sync::Arc;

use axum::http::HeaderMap;

use crate::app::AppState;

/// Native clients send no Origin header; browser contexts must match the
/// localhost/tauri defaults or the configured allowlist.
pub(super) fn origin_allowed(app: &Arc<AppState>, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        return true;
    };
    if origin == "null"
        || origin.starts_with("tauri://")
        || origin.starts_with("http://tauri.")
        || origin.starts_with("http://localhost")
        || origin.starts_with("http://127.0.0.1")
    {
        return true;
    }
    app.cfg.allowed_origins.iter().any(|o| o == origin)
}
