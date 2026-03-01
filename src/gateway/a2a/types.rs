//! A2A protocol data types.

use serde::{Deserialize, Serialize};
use std::time::Instant;

// ══════════════════════════════════════════════════════════════════
// JSON-RPC 2.0 Types
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: serde_json::Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

// JSON-RPC 2.0 standard error codes
pub const JSONRPC_PARSE_ERROR: i32 = -32700;
pub const JSONRPC_INVALID_REQUEST: i32 = -32600;
pub const JSONRPC_METHOD_NOT_FOUND: i32 = -32601;
pub const JSONRPC_INVALID_PARAMS: i32 = -32602;
pub const JSONRPC_INTERNAL_ERROR: i32 = -32603;

// A2A-specific error codes
pub const A2A_TASK_NOT_FOUND: i32 = -32001;
pub const A2A_TASK_NOT_CANCELABLE: i32 = -32002;

// ══════════════════════════════════════════════════════════════════
// A2A Protocol Types
// ══════════════════════════════════════════════════════════════════

/// Agent Card — returned by `GET /.well-known/agent.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub version: String,
    pub capabilities: AgentCapabilities,
    pub authentication: AgentAuthentication,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: Vec<AgentSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
    pub state_transition_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAuthentication {
    pub schemes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Task status per A2A spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Submitted,
    Working,
    Completed,
    Failed,
    Canceled,
}

/// A message part (text only for now).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Part {
    #[serde(rename = "text")]
    Text { text: String },
}

/// A message in the A2A conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub parts: Vec<Part>,
}

/// An artifact produced by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub parts: Vec<Part>,
    #[serde(default)]
    pub index: u32,
}

/// An A2A task.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATask {
    pub id: String,
    pub status: TaskStatusInfo,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Message>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskStatusInfo {
    pub state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
}

/// Internal task record with metadata for TTL eviction.
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub task: A2ATask,
    pub created_at: Instant,
    pub completed_at: Option<Instant>,
}

/// Params for `message/send`.
#[derive(Debug, Deserialize)]
pub struct MessageSendParams {
    pub id: String,
    pub message: Message,
}

/// Params for `tasks/get`.
#[derive(Debug, Deserialize)]
pub struct TasksGetParams {
    pub id: String,
}

/// Params for `tasks/cancel`.
#[derive(Debug, Deserialize)]
pub struct TasksCancelParams {
    pub id: String,
}
