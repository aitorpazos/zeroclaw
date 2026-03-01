//! A2A v1.0.0-rc HTTP handlers.
//!
//! - `GET /.well-known/agent-card.json` — Agent Card discovery (v1)
//! - `POST /a2a` — JSON-RPC 2.0 endpoint with v1 method names:
//!   - `SendMessage` — send a message and get a response
//!   - `GetTask` — retrieve task status and history
//!   - `CancelTask` — cancel a running task
//!   - `ListTasks` — list tasks with cursor-based pagination
//!
//! Reference: <https://a2a-protocol.org/latest/specification/>
//!
//! To remove v1 support, delete the `v1/` directory and remove references in `a2a/mod.rs`.

use super::types::*;
use crate::gateway::a2a::jsonrpc::{JsonRpcRequest, JsonRpcResponse};
use crate::gateway::a2a::store::CancelError;
use crate::gateway::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
};

/// GET `/.well-known/agent-card.json` — serves the v1 Agent Card.
pub async fn handle_agent_card_v1(State(state): State<AppState>) -> impl IntoResponse {
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
            description: Some("General-purpose AI assistant".into()),
            tags: Some(vec!["general".into(), "chat".into()]),
            input_modes: Some(vec!["text/plain".into()]),
            output_modes: Some(vec!["text/plain".into()]),
        }]
    } else {
        a2a.skills
            .iter()
            .map(|s| AgentSkill {
                id: s.id.clone(),
                name: s.name.clone(),
                description: if s.description.is_empty() {
                    None
                } else {
                    Some(s.description.clone())
                },
                tags: if s.tags.is_empty() {
                    None
                } else {
                    Some(s.tags.clone())
                },
                input_modes: Some(vec!["text/plain".into()]),
                output_modes: Some(vec!["text/plain".into()]),
            })
            .collect()
    };

    let card = AgentCard {
        name: a2a.name.clone(),
        description: Some(a2a.description.clone()),
        protocol_version: "1.0.0-rc".into(),
        supported_interfaces: vec![SupportedInterface {
            interface_type: "jsonrpc".into(),
            url: format!("{base_url}/a2a"),
        }],
        capabilities: AgentCapabilities {
            streaming: false,
            push_notifications: false,
            multi_turn: Some(true),
        },
        skills,
        default_input_modes: Some(vec!["text/plain".into()]),
        default_output_modes: Some(vec!["text/plain".into()]),
    };

    Json(card)
}

/// POST `/a2a` — JSON-RPC 2.0 dispatcher for v1 methods.
pub async fn handle_a2a_rpc_v1(
    state: &AppState,
    req: JsonRpcRequest,
) -> (StatusCode, Json<JsonRpcResponse>) {
    let response = match req.method.as_str() {
        "SendMessage" => handle_send_message(state, req.id.clone(), req.params).await,
        "GetTask" => handle_get_task(state, req.id.clone(), req.params),
        "CancelTask" => handle_cancel_task(state, req.id.clone(), req.params),
        "ListTasks" => handle_list_tasks(state, req.id.clone(), req.params),
        _ => JsonRpcResponse::error(
            req.id,
            METHOD_NOT_FOUND,
            format!("Method not found: {}", req.method),
        ),
    };

    (StatusCode::OK, Json(response))
}

/// Handle `SendMessage` (v1).
async fn handle_send_message(
    state: &AppState,
    id: Option<serde_json::Value>,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let send_params: SendMessageParams = match serde_json::from_value(params) {
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
            Part::Text { text, .. } => Some(text.as_str()),
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
            tracing::error!("A2A v1 task {task_id} failed: {e:#}");
            task_store.fail_task(&task_id, &format!("Agent error: {e}"));
        }
    }

    match task_store.get_task(&task_id, None) {
        Some(task) => {
            let v1_task = A2ATask::from_store(&task);
            JsonRpcResponse::success(id, serde_json::to_value(&v1_task).unwrap_or_default())
        }
        None => JsonRpcResponse::error(id, INTERNAL_ERROR, "Task disappeared unexpectedly"),
    }
}

/// Handle `GetTask` (v1).
fn handle_get_task(
    state: &AppState,
    id: Option<serde_json::Value>,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let get_params: GetTaskParams = match serde_json::from_value(params) {
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
            let v1_task = A2ATask::from_store(&task);
            JsonRpcResponse::success(id, serde_json::to_value(&v1_task).unwrap_or_default())
        }
        None => JsonRpcResponse::error(
            id,
            TASK_NOT_FOUND,
            format!("Task not found: {}", get_params.id),
        ),
    }
}

/// Handle `CancelTask` (v1).
fn handle_cancel_task(
    state: &AppState,
    id: Option<serde_json::Value>,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let cancel_params: CancelTaskParams = match serde_json::from_value(params) {
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
            let v1_task = A2ATask::from_store(&task);
            JsonRpcResponse::success(id, serde_json::to_value(&v1_task).unwrap_or_default())
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

/// Handle `ListTasks` (v1) — cursor-based pagination.
fn handle_list_tasks(
    state: &AppState,
    id: Option<serde_json::Value>,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let list_params: ListTasksParams = match serde_json::from_value(params) {
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

    let limit = list_params.limit.unwrap_or(20).min(100);
    let (tasks, next_cursor) = task_store.list_tasks(
        list_params.session_id.as_deref(),
        list_params.cursor.as_deref(),
        limit,
    );

    let v1_tasks: Vec<A2ATask> = tasks.iter().map(A2ATask::from_store).collect();

    let result = TaskListResult {
        tasks: v1_tasks,
        next_cursor,
    };

    JsonRpcResponse::success(id, serde_json::to_value(&result).unwrap_or_default())
}
