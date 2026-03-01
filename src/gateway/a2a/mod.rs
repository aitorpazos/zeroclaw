//! A2A (Agent-to-Agent) protocol module.
//!
//! Implements the Google A2A protocol for agent interoperability:
//! - Agent Card discovery (`GET /.well-known/agent.json`)
//! - JSON-RPC 2.0 endpoint (`POST /a2a`) with methods:
//!   - `message/send` — send a message and get a response
//!   - `tasks/get` — retrieve task status and history
//!   - `tasks/cancel` — cancel a running task

pub mod handlers;
pub mod store;
pub mod types;

pub use handlers::{handle_a2a_rpc, handle_agent_card};
pub use store::TaskStore;
