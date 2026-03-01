//! A2A protocol v1.0.0-rc — latest version support.
//!
//! Implements the A2A v1.0.0-rc specification:
//! - SCREAMING_SNAKE_CASE enum variants (`TASK_STATE_SUBMITTED`, `ROLE_USER`, etc.)
//! - Untagged `Part` types (TextPart, FilePart, DataPart distinguished by fields)
//! - Method names: `SendMessage`, `GetTask`, `CancelTask`, `ListTasks`
//! - Agent Card at `/.well-known/agent-card.json` with `supportedInterfaces`
//! - `createdAt` / `lastModified` timestamps on tasks
//!
//! Reference: <https://a2a-protocol.org/latest/specification/>

pub mod handlers;
pub mod types;

pub use handlers::{handle_a2a_rpc_v1, handle_agent_card_v1};
