//! A2A (Agent-to-Agent) protocol types.
//!
//! Implements the Google A2A protocol JSON-RPC 2.0 types, Agent Card,
//! and Task/Message/Artifact structures per the specification.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

// ── JSON-RPC 2.0 ────────────────────────────────────────────────

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// JSON-RPC 2.0 success response.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<serde_json::Value>, code: i64, message: impl Into<String>) -> Self {
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

// ── Standard JSON-RPC error codes ───────────────────────────────

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

// ── A2A-specific error codes ────────────────────────────────────

pub const TASK_NOT_FOUND: i64 = -32001;
pub const TASK_NOT_CANCELABLE: i64 = -32002;
pub const CAPACITY_EXCEEDED: i64 = -32003;

// ── Agent Card ──────────────────────────────────────────────────

/// Agent Card served at `/.well-known/agent.json`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub version: String,
    pub capabilities: AgentCapabilities,
    pub skills: Vec<AgentSkill>,
}

/// Capabilities advertised in the Agent Card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
    pub state_transition_history: bool,
}

/// Skill definition in the Agent Card.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

// ── Task Types ──────────────────────────────────────────────────

/// Task state machine per A2A spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Submitted,
    Working,
    #[serde(rename = "input-required")]
    InputRequired,
    Completed,
    Canceled,
    Failed,
}

/// A2A Task — the core unit of work.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATask {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Message>,
    #[serde(skip)]
    pub created_at: SystemTime,
}

/// Task status with state and optional message.
#[derive(Debug, Clone, Serialize)]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
}

/// A2A Message — a conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub parts: Vec<Part>,
}

/// Message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Agent,
}

/// Message part — text or data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Part {
    Text { text: String },
    Data { data: serde_json::Value },
}

/// Artifact produced by a task.
#[derive(Debug, Clone, Serialize)]
pub struct Artifact {
    pub name: String,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

// ── RPC Params ──────────────────────────────────────────────────

/// Parameters for `message/send`.
#[derive(Debug, Clone, Deserialize)]
pub struct MessageSendParams {
    pub message: Message,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
}

/// Parameters for `tasks/get`.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskGetParams {
    pub id: String,
    #[serde(default, rename = "historyLength")]
    pub history_length: Option<usize>,
}

/// Parameters for `tasks/cancel`.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskCancelParams {
    pub id: String,
}
