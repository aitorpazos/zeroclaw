//! Google A2A (Agent-to-Agent) protocol implementation.
//!
//! Implements the A2A protocol specification:
//! - `GET /.well-known/agent.json` — Agent Card discovery
//! - `POST /a2a` — JSON-RPC 2.0 message endpoint
//!
//! Gated behind the `a2a` compile-time feature flag.

mod handlers;
mod store;
mod types;

pub use handlers::{handle_a2a_rpc, handle_agent_card};
pub use store::A2ATaskStore;
pub use types::*;
