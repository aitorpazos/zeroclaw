//! A2A protocol v0.x (draft/0.2.x) — legacy version support.
//!
//! This module preserves the original A2A implementation with:
//! - camelCase enum variants (`submitted`, `working`, `completed`, etc.)
//! - Tagged `Part` type with `type` discriminator (`{"type": "text", "text": "..."}`)
//! - Method names: `message/send`, `tasks/get`, `tasks/cancel`
//! - Agent Card at `/.well-known/agent.json`

pub mod handlers;
pub mod types;

pub use handlers::{handle_a2a_rpc_v0, handle_agent_card_v0};
