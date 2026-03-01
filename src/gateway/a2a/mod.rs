//! A2A (Agent-to-Agent) protocol module — multi-version support.
//!
//! Supports multiple A2A protocol versions with per-version source files.
//! Version negotiation uses the `A2A-Protocol-Version` header, defaulting to v1.0.0-rc.
//!
//! ## Module structure
//!
//! ```text
//! a2a/
//! ├── mod.rs       ← This file: version negotiation, unified router entry points
//! ├── jsonrpc.rs   ← JSON-RPC 2.0 types (shared across all versions)
//! ├── store.rs     ← Version-independent task store
//! ├── v0/          ← A2A draft/0.2.x support
//! │   ├── mod.rs
//! │   ├── types.rs
//! │   └── handlers.rs
//! └── v1/          ← A2A v1.0.0-rc support (default)
//!     ├── mod.rs
//!     ├── types.rs
//!     └── handlers.rs
//! ```
//!
//! ## Adding/removing version support
//!
//! To add a new version (e.g. v2):
//! 1. Create `a2a/v2/` with `mod.rs`, `types.rs`, `handlers.rs`
//! 2. Add `pub mod v2;` below
//! 3. Add version matching in `detect_version()` and `handle_a2a_rpc()`
//!
//! To remove a version (e.g. v0):
//! 1. Delete the `a2a/v0/` directory
//! 2. Remove `pub mod v0;` below
//! 3. Remove version matching in `detect_version()` and `handle_a2a_rpc()`

pub mod jsonrpc;
pub mod store;
pub mod v0;
pub mod v1;

pub use store::TaskStore;

use crate::gateway::AppState;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use jsonrpc::{JsonRpcRequest, JsonRpcResponse, INVALID_REQUEST, PARSE_ERROR};

// ── Version Detection ───────────────────────────────────────────

/// Detected A2A protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A2AVersion {
    /// Draft / 0.2.x — original implementation
    V0,
    /// v1.0.0-rc — latest specification
    V1,
}

impl A2AVersion {
    /// Detect version from request headers and/or JSON-RPC method name.
    ///
    /// Priority:
    /// 1. `A2A-Protocol-Version` header (explicit)
    /// 2. JSON-RPC method name heuristic (PascalCase = v1, slash-separated = v0)
    /// 3. Default: v1.0.0-rc
    pub fn detect(headers: &HeaderMap, method: Option<&str>) -> Self {
        // 1. Check explicit header
        if let Some(version_header) = headers.get("a2a-protocol-version") {
            if let Ok(v) = version_header.to_str() {
                return Self::from_version_string(v);
            }
        }

        // 2. Infer from method name
        if let Some(method) = method {
            return Self::from_method_name(method);
        }

        // 3. Default to v1
        Self::V1
    }

    /// Parse a version string (e.g. "0.2.1", "1.0.0-rc", "1.0.0").
    fn from_version_string(version: &str) -> Self {
        let trimmed = version.trim();
        if trimmed.starts_with("0.") {
            Self::V0
        } else {
            // "1.0.0-rc", "1.0.0", "1.x", or anything else → v1
            Self::V1
        }
    }

    /// Infer version from JSON-RPC method name.
    ///
    /// v0 methods: `message/send`, `tasks/get`, `tasks/cancel`
    /// v1 methods: `SendMessage`, `GetTask`, `CancelTask`, `ListTasks`
    fn from_method_name(method: &str) -> Self {
        match method {
            "message/send" | "tasks/get" | "tasks/cancel" => Self::V0,
            "SendMessage" | "GetTask" | "CancelTask" | "ListTasks" => Self::V1,
            _ => Self::V1, // Unknown methods default to v1
        }
    }
}

// ── Unified Handlers ────────────────────────────────────────────

/// GET `/.well-known/agent.json` — v0 Agent Card (legacy endpoint).
pub async fn handle_agent_card_v0(state: State<AppState>) -> impl IntoResponse {
    v0::handle_agent_card_v0(state).await
}

/// GET `/.well-known/agent-card.json` — v1 Agent Card.
pub async fn handle_agent_card_v1(state: State<AppState>) -> impl IntoResponse {
    v1::handle_agent_card_v1(state).await
}

/// POST `/a2a` — version-negotiated JSON-RPC 2.0 dispatcher.
///
/// Detects the protocol version from headers and method name, then routes
/// to the appropriate version handler.
pub async fn handle_a2a_rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<JsonRpcRequest>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("A2A JSON parse error: {e}");
            return (
                StatusCode::OK,
                Json(JsonRpcResponse::error(
                    None,
                    PARSE_ERROR,
                    format!("Parse error: {e}"),
                )),
            );
        }
    };

    if req.jsonrpc != "2.0" {
        return (
            StatusCode::OK,
            Json(JsonRpcResponse::error(
                req.id,
                INVALID_REQUEST,
                "Invalid JSON-RPC version, expected \"2.0\"",
            )),
        );
    }

    let version = A2AVersion::detect(&headers, Some(&req.method));

    match version {
        A2AVersion::V0 => {
            tracing::debug!("A2A request routed to v0 handler: {}", req.method);
            v0::handle_a2a_rpc_v0(&state, req).await
        }
        A2AVersion::V1 => {
            tracing::debug!("A2A request routed to v1 handler: {}", req.method);
            v1::handle_a2a_rpc_v1(&state, req).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn test_version_detection_from_header() {
        let mut headers = HeaderMap::new();
        headers.insert("a2a-protocol-version", HeaderValue::from_static("0.2.1"));
        assert_eq!(A2AVersion::detect(&headers, None), A2AVersion::V0);

        let mut headers = HeaderMap::new();
        headers.insert("a2a-protocol-version", HeaderValue::from_static("1.0.0-rc"));
        assert_eq!(A2AVersion::detect(&headers, None), A2AVersion::V1);

        let mut headers = HeaderMap::new();
        headers.insert("a2a-protocol-version", HeaderValue::from_static("1.0.0"));
        assert_eq!(A2AVersion::detect(&headers, None), A2AVersion::V1);
    }

    #[test]
    fn test_version_detection_from_method() {
        let headers = HeaderMap::new();

        // v0 methods
        assert_eq!(
            A2AVersion::detect(&headers, Some("message/send")),
            A2AVersion::V0
        );
        assert_eq!(
            A2AVersion::detect(&headers, Some("tasks/get")),
            A2AVersion::V0
        );
        assert_eq!(
            A2AVersion::detect(&headers, Some("tasks/cancel")),
            A2AVersion::V0
        );

        // v1 methods
        assert_eq!(
            A2AVersion::detect(&headers, Some("SendMessage")),
            A2AVersion::V1
        );
        assert_eq!(
            A2AVersion::detect(&headers, Some("GetTask")),
            A2AVersion::V1
        );
        assert_eq!(
            A2AVersion::detect(&headers, Some("CancelTask")),
            A2AVersion::V1
        );
        assert_eq!(
            A2AVersion::detect(&headers, Some("ListTasks")),
            A2AVersion::V1
        );
    }

    #[test]
    fn test_version_detection_header_takes_priority() {
        let mut headers = HeaderMap::new();
        headers.insert("a2a-protocol-version", HeaderValue::from_static("0.2.1"));
        // Even with a v1 method name, header wins
        assert_eq!(
            A2AVersion::detect(&headers, Some("SendMessage")),
            A2AVersion::V0
        );
    }

    #[test]
    fn test_version_detection_defaults_to_v1() {
        let headers = HeaderMap::new();
        assert_eq!(A2AVersion::detect(&headers, None), A2AVersion::V1);
        assert_eq!(
            A2AVersion::detect(&headers, Some("UnknownMethod")),
            A2AVersion::V1
        );
    }
}
