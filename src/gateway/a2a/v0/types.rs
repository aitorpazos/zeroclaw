//! A2A v0.x (draft/0.2.x) types.
//!
//! Original A2A types with camelCase enums and tagged Part discriminator.
//! To remove v0 support, delete the `v0/` directory and remove references in `a2a/mod.rs`.

use serde::{Deserialize, Serialize};

// ── JSON-RPC 2.0 (shared across versions) ──────────────────────

pub use super::super::jsonrpc::{
    CAPACITY_EXCEEDED, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND, TASK_NOT_CANCELABLE,
    TASK_NOT_FOUND,
};

// ── Agent Card (v0) ─────────────────────────────────────────────

/// Agent Card served at `/.well-known/agent.json` (v0.x).
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

/// Capabilities advertised in the v0 Agent Card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
    pub state_transition_history: bool,
}

/// Skill definition in the v0 Agent Card.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

// ── Task Types (v0) ─────────────────────────────────────────────

/// Task state machine per A2A v0 spec (camelCase variants).
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

impl From<crate::gateway::a2a::store::TaskState> for TaskState {
    fn from(s: crate::gateway::a2a::store::TaskState) -> Self {
        match s {
            crate::gateway::a2a::store::TaskState::Submitted => Self::Submitted,
            crate::gateway::a2a::store::TaskState::Working => Self::Working,
            crate::gateway::a2a::store::TaskState::InputRequired => Self::InputRequired,
            crate::gateway::a2a::store::TaskState::Completed => Self::Completed,
            crate::gateway::a2a::store::TaskState::Canceled => Self::Canceled,
            crate::gateway::a2a::store::TaskState::Failed => Self::Failed,
            // v0 doesn't have Rejected/AuthRequired — map to Failed
            crate::gateway::a2a::store::TaskState::Rejected => Self::Failed,
            crate::gateway::a2a::store::TaskState::AuthRequired => Self::Failed,
            crate::gateway::a2a::store::TaskState::Unknown => Self::Failed,
        }
    }
}

/// A2A Task — the core unit of work (v0 serialization format).
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
}

/// Task status with state and optional message (v0).
#[derive(Debug, Clone, Serialize)]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
}

/// A2A Message — a conversation turn (v0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub parts: Vec<Part>,
}

/// Message role (v0: lowercase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Agent,
}

/// Message part — tagged with `type` discriminator (v0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Part {
    Text { text: String },
    Data { data: serde_json::Value },
}

/// Artifact produced by a task (v0).
#[derive(Debug, Clone, Serialize)]
pub struct Artifact {
    pub name: String,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

// ── Conversions from store types ────────────────────────────────

impl A2ATask {
    /// Convert from the version-independent store task to v0 wire format.
    pub fn from_store(task: &crate::gateway::a2a::store::StoredTask) -> Self {
        Self {
            id: task.id.clone(),
            session_id: task.session_id.clone(),
            status: TaskStatus {
                state: task.state.into(),
                message: task.status_message.as_ref().map(|m| Message {
                    role: MessageRole::Agent,
                    parts: vec![Part::Text {
                        text: m.clone(),
                    }],
                }),
            },
            artifacts: task
                .artifacts
                .iter()
                .enumerate()
                .map(|(i, a)| Artifact {
                    name: a.name.clone(),
                    parts: a
                        .parts
                        .iter()
                        .map(|p| Part::Text {
                            text: p.clone(),
                        })
                        .collect(),
                    index: Some(i as u32),
                })
                .collect(),
            history: task
                .history
                .iter()
                .map(|h| Message {
                    role: if h.is_agent {
                        MessageRole::Agent
                    } else {
                        MessageRole::User
                    },
                    parts: vec![Part::Text {
                        text: h.text.clone(),
                    }],
                })
                .collect(),
        }
    }
}

// ── RPC Params (v0) ─────────────────────────────────────────────

/// Parameters for `message/send` (v0).
#[derive(Debug, Clone, Deserialize)]
pub struct MessageSendParams {
    pub message: Message,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
}

/// Parameters for `tasks/get` (v0).
#[derive(Debug, Clone, Deserialize)]
pub struct TaskGetParams {
    pub id: String,
    #[serde(default, rename = "historyLength")]
    pub history_length: Option<usize>,
}

/// Parameters for `tasks/cancel` (v0).
#[derive(Debug, Clone, Deserialize)]
pub struct TaskCancelParams {
    pub id: String,
}
