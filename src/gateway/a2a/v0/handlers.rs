//! A2A v0.x (draft/0.2.x) HTTP handlers.
//!
//! - `GET /.well-known/agent.json` — Agent Card discovery (v0)
//! - `POST /a2a` — JSON-RPC 2.0 endpoint with v0 method names:
//!   - `message/send` — send a message and get a response
//!   - `tasks/get` — retrieve task status and history
//!   - `tasks/cancel` — cancel a running task
//!
//! To remove v0 support, delete the `v0/` directory and remove references in `a2a/mod.rs`.

use super::types::*;
use crate::gateway::a2a::jsonrpc::{JsonRpcRequest, JsonRpcResponse};
use crate::gateway::a2a::store::CancelError;
use crate::gateway::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
};

/// GET `/.well-known/agent.json` — serves the v0 Agent Card.
pub async fn handle_agent_card_v0(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock();
    let a2a = &config.gateway.a2a;

    let base_url = a2a
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://{}:{}", config.gateway.host, config.gateway.port));

    let skills: Vec<AgentSkill> = if a2a.skills.is_empty() {
        vec![AgentSkill {
            id: "general".into(),
            name: "General Assistant".into(),
            description: "General-purpose AI assistant".into(),
            tags: vec!["general".into(), "chat".into()],
        }]
    } else {
        a2a.skills
            .iter()
            .map(|s| AgentSkill {
                id: s.id.clone(),
                name: s.name.clone(),
                description: s.description.clone(),
                tags: s.tags.clone(),
            })
            .collect()
    };

    let card = AgentCard {
        name: a2a.name.clone(),
        description: a2a.description.clone(),
        url: format!("{base_url}/a2a"),
        version: a2a.version.clone(),
        capabilities: AgentCapabilities {
            streaming: false,
            push_notifications: false,
            state_transition_history: true,
        },
        skills,
    };

    Json(card)
}

/// POST `/a2a` — JSON-RPC 2.0 dispatcher for v0 methods.
pub async fn handle_a2a_rpc_v0(
    state: &AppState,
    req: JsonRpcRequest,
) -> (StatusCode, Json<JsonRpcResponse>) {
    let response = match req.method.as_str() {
        "message/send" => handle_message_send(state, req.id.clone(), req.params).await,
        "tasks/get" => handle_tasks_get(state, req.id.clone(), req.params),
        "tasks/cancel" => handle_tasks_cancel(state, req.id.clone(), req.params),
        _ => JsonRpcResponse::error(
            req.id,
            super::types::METHOD_NOT_FOUND,
            format!("Method not found: {}", req.method),
        ),
    };

    (StatusCode::OK, Json(response))
}

/// Handle `message/send` (v0).
async fn handle_message_send(
    state: &AppState,
    id: Option<serde_json::Value>,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let send_params: MessageSendParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                id,
                INVALID_PARAMS,
                format!("Invalid params: {e}"),
            );
        }
    };

    // Extract text from message parts
    let user_text: String = send_params
        .message
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if user_text.trim().is_empty() {
        return JsonRpcResponse::error(id, INVALID_PARAMS, "Message contains no text parts");
    }

    let task_store = match state.a2a_store.as_ref() {
        Some(store) => store,
        None => {
            return JsonRpcResponse::error(id, INTERNAL_ERROR, "A2A not initialized");
        }
    };

    let task = match task_store.create_task(
        send_params.id,
        send_params.session_id,
        &user_text,
    ) {
        Some(t) => t,
        None => {
            return JsonRpcResponse::error(
                id,
                CAPACITY_EXCEEDED,
                "Task capacity exceeded, try again later",
            );
        }
    };

    let task_id = task.id.clone();
    task_store.mark_working(&task_id);

    let session_ref = task.session_id.as_deref();
    match super::super::super::run_gateway_chat_with_tools(state, &user_text, session_ref).await {
        Ok(response) => {
            task_store.complete_task(&task_id, &response);
        }
        Err(e) => {
            tracing::error!("A2A v0 task {task_id} failed: {e:#}");
            task_store.fail_task(&task_id, &format!("Agent error: {e}"));
        }
    }

    match task_store.get_task(&task_id, None) {
        Some(task) => {
            let v0_task = A2ATask::from_store(&task);
            JsonRpcResponse::success(id, serde_json::to_value(&v0_task).unwrap_or_default())
        }
        None => JsonRpcResponse::error(id, INTERNAL_ERROR, "Task disappeared unexpectedly"),
    }
}

/// Handle `tasks/get` (v0).
fn handle_tasks_get(
    state: &AppState,
    id: Option<serde_json::Value>,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let get_params: TaskGetParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                id,
                INVALID_PARAMS,
                format!("Invalid params: {e}"),
            );
        }
    };

    let task_store = match state.a2a_store.as_ref() {
        Some(store) => store,
        None => {
            return JsonRpcResponse::error(id, INTERNAL_ERROR, "A2A not initialized");
        }
    };

    match task_store.get_task(&get_params.id, get_params.history_length) {
        Some(task) => {
            let v0_task = A2ATask::from_store(&task);
            JsonRpcResponse::success(id, serde_json::to_value(&v0_task).unwrap_or_default())
        }
        None => JsonRpcResponse::error(
            id,
            TASK_NOT_FOUND,
            format!("Task not found: {}", get_params.id),
        ),
    }
}

/// Handle `tasks/cancel` (v0).
fn handle_tasks_cancel(
    state: &AppState,
    id: Option<serde_json::Value>,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let cancel_params: TaskCancelParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                id,
                INVALID_PARAMS,
                format!("Invalid params: {e}"),
            );
        }
    };

    let task_store = match state.a2a_store.as_ref() {
        Some(store) => store,
        None => {
            return JsonRpcResponse::error(id, INTERNAL_ERROR, "A2A not initialized");
        }
    };

    match task_store.cancel_task(&cancel_params.id) {
        Ok(task) => {
            let v0_task = A2ATask::from_store(&task);
            JsonRpcResponse::success(id, serde_json::to_value(&v0_task).unwrap_or_default())
        }
        Err(CancelError::NotFound) => JsonRpcResponse::error(
            id,
            TASK_NOT_FOUND,
            format!("Task not found: {}", cancel_params.id),
        ),
        Err(CancelError::NotCancelable(state)) => JsonRpcResponse::error(
            id,
            TASK_NOT_CANCELABLE,
            format!(
                "Task {} is in terminal state {:?} and cannot be canceled",
                cancel_params.id, state
            ),
        ),
    }
}
