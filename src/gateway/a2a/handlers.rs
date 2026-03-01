//! A2A protocol HTTP handlers.

use super::types::*;
use crate::gateway::{run_gateway_chat_with_tools, sanitize_gateway_response, AppState};
use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};

/// `GET /.well-known/agent.json` — Agent Card discovery endpoint (public, no auth).
pub async fn handle_agent_card(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock();
    let a2a_config = &config.gateway.a2a;

    if !a2a_config.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "A2A protocol is not enabled"})),
        );
    }

    let url = a2a_config
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://{}:{}/a2a", config.gateway.host, config.gateway.port));

    let skills: Vec<AgentSkill> = if a2a_config.skills.is_empty() {
        vec![AgentSkill {
            id: "general-assistant".into(),
            name: "General Assistant".into(),
            description: "General-purpose AI assistant with tool access".into(),
            tags: vec!["general".into(), "assistant".into()],
        }]
    } else {
        a2a_config
            .skills
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
        name: a2a_config.name.clone(),
        description: a2a_config.description.clone(),
        url,
        version: a2a_config.version.clone(),
        capabilities: AgentCapabilities {
            streaming: false,
            push_notifications: false,
            state_transition_history: false,
        },
        authentication: AgentAuthentication {
            schemes: if config.gateway.require_pairing {
                vec!["bearer".into()]
            } else {
                vec![]
            },
        },
        default_input_modes: vec!["text".into()],
        default_output_modes: vec!["text".into()],
        skills,
    };

    (StatusCode::OK, Json(serde_json::to_value(card).unwrap()))
}

/// `POST /a2a` — JSON-RPC 2.0 endpoint for A2A protocol.
pub async fn handle_a2a_rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // ── Check A2A enabled ──
    {
        let config = state.config.lock();
        if !config.gateway.a2a.enabled {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "A2A protocol is not enabled"})),
            );
        }
    }

    // ── Bearer auth (pairing) ──
    if state.pairing.require_pairing() {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let token = auth.strip_prefix("Bearer ").unwrap_or("").trim();
        if !state.pairing.is_authenticated(token) {
            let resp = JsonRpcResponse::error(
                serde_json::Value::Null,
                JSONRPC_INTERNAL_ERROR,
                "Unauthorized — pair first via POST /pair",
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::to_value(resp).unwrap()),
            );
        }
    }

    // ── Parse JSON-RPC request ──
    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => {
            tracing::warn!("A2A JSON-RPC parse error: {e}");
            let resp = JsonRpcResponse::error(
                serde_json::Value::Null,
                JSONRPC_PARSE_ERROR,
                "Parse error",
            );
            return (StatusCode::OK, Json(serde_json::to_value(resp).unwrap()));
        }
    };

    // ── Validate JSON-RPC version ──
    if request.jsonrpc != "2.0" {
        let resp = JsonRpcResponse::error(
            request.id,
            JSONRPC_INVALID_REQUEST,
            "Invalid JSON-RPC version, expected 2.0",
        );
        return (StatusCode::OK, Json(serde_json::to_value(resp).unwrap()));
    }

    // ── Route to method handler ──
    let response = match request.method.as_str() {
        "message/send" => handle_message_send(&state, request.id.clone(), request.params).await,
        "tasks/get" => handle_tasks_get(&state, request.id.clone(), request.params),
        "tasks/cancel" => handle_tasks_cancel(&state, request.id.clone(), request.params),
        _ => JsonRpcResponse::error(
            request.id,
            JSONRPC_METHOD_NOT_FOUND,
            format!("Method not found: {}", request.method),
        ),
    };

    (StatusCode::OK, Json(serde_json::to_value(response).unwrap()))
}

// ── Method: message/send ──

async fn handle_message_send(
    state: &AppState,
    rpc_id: serde_json::Value,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let params: MessageSendParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                rpc_id,
                JSONRPC_INVALID_PARAMS,
                format!("Invalid params: {e}"),
            );
        }
    };

    // Extract text from message parts
    let user_text: String = params
        .message
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
        })
        .collect::<Vec<_>>()
        .join("\n");

    if user_text.trim().is_empty() {
        return JsonRpcResponse::error(
            rpc_id,
            JSONRPC_INVALID_PARAMS,
            "Message must contain at least one non-empty text part",
        );
    }

    let task_store = &state.a2a_task_store;

    // Create or resume task
    if !task_store.task_exists(&params.id) {
        task_store.create_task(&params.id, params.message.clone());
    } else {
        task_store.add_to_history(&params.id, params.message.clone());
    }

    // Transition to working
    task_store.set_working(&params.id);

    tracing::info!(
        "A2A message/send: task={}, text={}",
        params.id,
        crate::util::truncate_with_ellipsis(&user_text, 80)
    );

    // Run through the full agent loop
    match run_gateway_chat_with_tools(state, &user_text).await {
        Ok(response) => {
            let safe_response =
                sanitize_gateway_response(&response, state.tools_registry_exec.as_ref());

            let response_message = Message {
                role: "agent".into(),
                parts: vec![Part::Text {
                    text: safe_response.clone(),
                }],
            };

            task_store.set_completed(&params.id, safe_response, response_message);

            let task = task_store.get_task(&params.id).unwrap();
            JsonRpcResponse::success(rpc_id, serde_json::to_value(task).unwrap())
        }
        Err(e) => {
            let error_msg = format!("Agent processing failed: {e:#}");
            tracing::error!("A2A task {} failed: {error_msg}", params.id);
            task_store.set_failed(&params.id, &error_msg);

            let task = task_store.get_task(&params.id).unwrap();
            JsonRpcResponse::success(rpc_id, serde_json::to_value(task).unwrap())
        }
    }
}

// ── Method: tasks/get ──

fn handle_tasks_get(
    state: &AppState,
    rpc_id: serde_json::Value,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let params: TasksGetParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                rpc_id,
                JSONRPC_INVALID_PARAMS,
                format!("Invalid params: {e}"),
            );
        }
    };

    match state.a2a_task_store.get_task(&params.id) {
        Some(task) => JsonRpcResponse::success(rpc_id, serde_json::to_value(task).unwrap()),
        None => JsonRpcResponse::error(
            rpc_id,
            A2A_TASK_NOT_FOUND,
            format!("Task not found: {}", params.id),
        ),
    }
}

// ── Method: tasks/cancel ──

fn handle_tasks_cancel(
    state: &AppState,
    rpc_id: serde_json::Value,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let params: TasksCancelParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::error(
                rpc_id,
                JSONRPC_INVALID_PARAMS,
                format!("Invalid params: {e}"),
            );
        }
    };

    if !state.a2a_task_store.task_exists(&params.id) {
        return JsonRpcResponse::error(
            rpc_id,
            A2A_TASK_NOT_FOUND,
            format!("Task not found: {}", params.id),
        );
    }

    match state.a2a_task_store.set_canceled(&params.id) {
        Ok(()) => {
            tracing::info!("A2A task {} canceled", params.id);
            let task = state.a2a_task_store.get_task(&params.id).unwrap();
            JsonRpcResponse::success(rpc_id, serde_json::to_value(task).unwrap())
        }
        Err(reason) => JsonRpcResponse::error(rpc_id, A2A_TASK_NOT_CANCELABLE, reason),
    }
}
