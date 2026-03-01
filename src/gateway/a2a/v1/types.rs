//! A2A v1.0.0-rc types.
//!
//! Implements the v1.0.0-rc specification with:
//! - SCREAMING_SNAKE_CASE enum variants
//! - Untagged Part types (TextPart, FilePart, DataPart)
//! - Agent Card with `supportedInterfaces`
//! - `createdAt` / `lastModified` timestamps
//!
//! Reference: <https://a2a-protocol.org/latest/specification/>
//!
//! To remove v1 support, delete the `v1/` directory and remove references in `a2a/mod.rs`.

use serde::{Deserialize, Serialize};

// ── JSON-RPC 2.0 (shared across versions) ──────────────────────

pub use super::super::jsonrpc::{
    CAPACITY_EXCEEDED, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND, TASK_NOT_CANCELABLE,
    TASK_NOT_FOUND,
};

// ── Agent Card (v1) ─────────────────────────────────────────────

/// Agent Card served at `/.well-known/agent-card.json` (v1.0.0-rc).
///
/// Per spec: The Agent Card is a JSON document that describes the agent's
/// capabilities, skills, and supported interfaces.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub protocol_version: String,
    pub supported_interfaces: Vec<SupportedInterface>,
    pub capabilities: AgentCapabilities,
    pub skills: Vec<AgentSkill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_input_modes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_output_modes: Option<Vec<String>>,
}

/// Supported interface (transport) in the v1 Agent Card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedInterface {
    #[serde(rename = "type")]
    pub interface_type: String,
    pub url: String,
}

/// Capabilities advertised in the v1 Agent Card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_turn: Option<bool>,
}

/// Skill definition in the v1 Agent Card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_modes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_modes: Option<Vec<String>>,
}

// ── Task Types (v1) ─────────────────────────────────────────────

/// Task state machine per A2A v1.0.0-rc spec (SCREAMING_SNAKE_CASE).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    #[serde(rename = "submitted")]
    Submitted,
    #[serde(rename = "working")]
    Working,
    #[serde(rename = "input-required")]
    InputRequired,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "canceled")]
    Canceled,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "rejected")]
    Rejected,
    #[serde(rename = "auth-required")]
    AuthRequired,
    #[serde(rename = "unknown")]
    Unknown,
}

impl From<crate::gateway::a2a::store::TaskState> for TaskState {
    fn from(s: crate::gateway::a2a::store::TaskState) -> Self {
        match s {
            crate::gateway::a2a::store::TaskState::Submitted => Self::Submitted,
            crate::gateway::a2a::store::TaskState::Working => Self::Working,
            crate::gateway::a2a::store::TaskState::InputRequired => Self::InputRequired,
            crate::gateway::a2a::store::TaskState::Completed => Self::Completed,
            crate::gateway::a2a::store::TaskState::Canceled => Self::Canceled,
            crate::gateway::a2a::store::TaskState::Failed => Self::Failed,
            crate::gateway::a2a::store::TaskState::Rejected => Self::Rejected,
            crate::gateway::a2a::store::TaskState::AuthRequired => Self::AuthRequired,
            crate::gateway::a2a::store::TaskState::Unknown => Self::Unknown,
        }
    }
}

/// A2A Task — the core unit of work (v1 serialization format).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATask {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<Artifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<Message>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

/// Task status with state and optional message (v1).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// A2A Message — a conversation turn (v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: MessageRole,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Message role (v1: lowercase per spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Agent,
}

/// Message part — untagged, distinguished by fields present (v1).
///
/// Per v1.0.0-rc spec, parts are distinguished by which fields are present:
/// - TextPart: has `text` field
/// - FilePart: has `file` field
/// - DataPart: has `data` field
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text {
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
    },
    File {
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        file: FileContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
    },
    Data {
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        data: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
    },
}

/// File content in a FilePart (v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Inline bytes (base64-encoded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
    /// URI reference to the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// Artifact produced by a task (v1).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

// ── Conversions from store types ────────────────────────────────

impl A2ATask {
    /// Convert from the version-independent store task to v1 wire format.
    pub fn from_store(task: &crate::gateway::a2a::store::StoredTask) -> Self {
        let created_at = task
            .created_at
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default()
            });

        let last_modified = task
            .last_modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default()
            });

        Self {
            id: task.id.clone(),
            session_id: task.session_id.clone(),
            status: TaskStatus {
                state: task.state.into(),
                message: task.status_message.as_ref().map(|m| Message {
                    role: MessageRole::Agent,
                    parts: vec![Part::Text {
                        kind: Some("text".into()),
                        text: m.clone(),
                        metadata: None,
                    }],
                    metadata: None,
                }),
                timestamp: last_modified.clone(),
            },
            artifacts: if task.artifacts.is_empty() {
                None
            } else {
                Some(
                    task.artifacts
                        .iter()
                        .enumerate()
                        .map(|(i, a)| Artifact {
                            artifact_id: Some(format!("artifact-{i}")),
                            name: Some(a.name.clone()),
                            parts: a
                                .parts
                                .iter()
                                .map(|p| Part::Text {
                                    kind: Some("text".into()),
                                    text: p.clone(),
                                    metadata: None,
                                })
                                .collect(),
                            metadata: None,
                        })
                        .collect(),
                )
            },
            history: if task.history.is_empty() {
                None
            } else {
                Some(
                    task.history
                        .iter()
                        .map(|h| Message {
                            role: if h.is_agent {
                                MessageRole::Agent
                            } else {
                                MessageRole::User
                            },
                            parts: vec![Part::Text {
                                kind: Some("text".into()),
                                text: h.text.clone(),
                                metadata: None,
                            }],
                            metadata: None,
                        })
                        .collect(),
                )
            },
            created_at,
            last_modified,
        }
    }
}

// ── RPC Params (v1) ─────────────────────────────────────────────

/// Parameters for `SendMessage` (v1).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageParams {
    pub message: Message,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Parameters for `GetTask` (v1).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTaskParams {
    pub id: String,
    #[serde(default)]
    pub history_length: Option<usize>,
}

/// Parameters for `CancelTask` (v1).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelTaskParams {
    pub id: String,
}

/// Parameters for `ListTasks` (v1).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksParams {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Paginated task list response (v1).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListResult {
    pub tasks: Vec<A2ATask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
