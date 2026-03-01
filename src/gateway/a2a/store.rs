//! In-memory A2A task store with capacity limits and TTL eviction.

use super::types::{A2ATask, Artifact, Message, Part, TaskState, TaskStatus};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// Thread-safe in-memory task store.
#[derive(Clone)]
pub struct TaskStore {
    inner: Arc<Mutex<StoreInner>>,
}

struct StoreInner {
    tasks: HashMap<String, A2ATask>,
    max_tasks: usize,
    task_ttl: Duration,
}

impl TaskStore {
    /// Create a new task store with the given capacity and TTL.
    pub fn new(max_tasks: usize, task_ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StoreInner {
                tasks: HashMap::new(),
                max_tasks,
                task_ttl: Duration::from_secs(task_ttl_secs),
            })),
        }
    }

    /// Create a new task from a user message. Returns the task ID.
    ///
    /// Evicts expired tasks first, then checks capacity.
    /// Returns `None` if capacity is exceeded after eviction.
    pub fn create_task(
        &self,
        id: Option<String>,
        session_id: Option<String>,
        message: Message,
    ) -> Option<A2ATask> {
        let mut inner = self.inner.lock();
        self.evict_expired(&mut inner);

        if inner.tasks.len() >= inner.max_tasks {
            return None;
        }

        let task_id = id.unwrap_or_else(|| Uuid::new_v4().to_string());

        let task = A2ATask {
            id: task_id.clone(),
            session_id,
            status: TaskStatus {
                state: TaskState::Submitted,
                message: None,
            },
            artifacts: Vec::new(),
            history: vec![message],
            created_at: SystemTime::now(),
        };

        inner.tasks.insert(task_id, task.clone());
        Some(task)
    }

    /// Get a task by ID, optionally truncating history.
    pub fn get_task(&self, id: &str, history_length: Option<usize>) -> Option<A2ATask> {
        let inner = self.inner.lock();
        inner.tasks.get(id).map(|task| {
            let mut task = task.clone();
            if let Some(max_len) = history_length {
                let len = task.history.len();
                if len > max_len {
                    task.history = task.history[len - max_len..].to_vec();
                }
            }
            task
        })
    }

    /// Update task state to Working.
    pub fn mark_working(&self, id: &str) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(id) {
            task.status.state = TaskState::Working;
        }
    }

    /// Complete a task with the agent's response.
    pub fn complete_task(&self, id: &str, response_text: &str) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(id) {
            let agent_message = Message {
                role: super::types::MessageRole::Agent,
                parts: vec![Part::Text {
                    text: response_text.to_string(),
                }],
            };

            task.history.push(agent_message.clone());
            task.artifacts.push(Artifact {
                name: "response".into(),
                parts: vec![Part::Text {
                    text: response_text.to_string(),
                }],
                index: Some(0),
            });
            task.status = TaskStatus {
                state: TaskState::Completed,
                message: Some(agent_message),
            };
        }
    }

    /// Fail a task with an error message.
    pub fn fail_task(&self, id: &str, error: &str) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(id) {
            let error_message = Message {
                role: super::types::MessageRole::Agent,
                parts: vec![Part::Text {
                    text: error.to_string(),
                }],
            };

            task.history.push(error_message.clone());
            task.status = TaskStatus {
                state: TaskState::Failed,
                message: Some(error_message),
            };
        }
    }

    /// Cancel a task. Returns false if task doesn't exist or is already terminal.
    pub fn cancel_task(&self, id: &str) -> Result<A2ATask, CancelError> {
        let mut inner = self.inner.lock();
        let task = inner.tasks.get_mut(id).ok_or(CancelError::NotFound)?;

        match task.status.state {
            TaskState::Completed | TaskState::Canceled | TaskState::Failed => {
                Err(CancelError::NotCancelable(task.status.state))
            }
            _ => {
                task.status = TaskStatus {
                    state: TaskState::Canceled,
                    message: Some(Message {
                        role: super::types::MessageRole::Agent,
                        parts: vec![Part::Text {
                            text: "Task canceled by client".into(),
                        }],
                    }),
                };
                Ok(task.clone())
            }
        }
    }

    /// Evict tasks that have exceeded their TTL.
    fn evict_expired(&self, inner: &mut StoreInner) {
        let now = SystemTime::now();
        inner.tasks.retain(|_, task| {
            now.duration_since(task.created_at)
                .map(|age| age < inner.task_ttl)
                .unwrap_or(true)
        });
    }
}

/// Errors from cancel operations.
#[derive(Debug)]
pub enum CancelError {
    NotFound,
    NotCancelable(TaskState),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::a2a::types::{MessageRole, Part};

    fn test_message(text: &str) -> Message {
        Message {
            role: MessageRole::User,
            parts: vec![Part::Text {
                text: text.to_string(),
            }],
        }
    }

    #[test]
    fn test_create_and_get_task() {
        let store = TaskStore::new(10, 3600);
        let task = store
            .create_task(None, None, test_message("hello"))
            .unwrap();
        assert_eq!(task.status.state, TaskState::Submitted);
        assert_eq!(task.history.len(), 1);

        let fetched = store.get_task(&task.id, None).unwrap();
        assert_eq!(fetched.id, task.id);
    }

    #[test]
    fn test_task_lifecycle() {
        let store = TaskStore::new(10, 3600);
        let task = store
            .create_task(None, None, test_message("hello"))
            .unwrap();

        store.mark_working(&task.id);
        let t = store.get_task(&task.id, None).unwrap();
        assert_eq!(t.status.state, TaskState::Working);

        store.complete_task(&task.id, "world");
        let t = store.get_task(&task.id, None).unwrap();
        assert_eq!(t.status.state, TaskState::Completed);
        assert_eq!(t.history.len(), 2);
        assert_eq!(t.artifacts.len(), 1);
    }

    #[test]
    fn test_capacity_limit() {
        let store = TaskStore::new(2, 3600);
        store
            .create_task(None, None, test_message("one"))
            .unwrap();
        store
            .create_task(None, None, test_message("two"))
            .unwrap();
        assert!(store
            .create_task(None, None, test_message("three"))
            .is_none());
    }

    #[test]
    fn test_ttl_eviction() {
        let store = TaskStore::new(10, 0); // 0s TTL = immediate expiry
        store
            .create_task(None, None, test_message("ephemeral"))
            .unwrap();

        // Sleep briefly to ensure expiry
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Next create triggers eviction
        let task = store
            .create_task(None, None, test_message("new"))
            .unwrap();
        assert!(store.get_task(&task.id, None).is_some());
    }

    #[test]
    fn test_cancel_task() {
        let store = TaskStore::new(10, 3600);
        let task = store
            .create_task(None, None, test_message("cancel me"))
            .unwrap();

        store.mark_working(&task.id);
        let canceled = store.cancel_task(&task.id).unwrap();
        assert_eq!(canceled.status.state, TaskState::Canceled);

        // Can't cancel again
        assert!(store.cancel_task(&task.id).is_err());
    }

    #[test]
    fn test_cancel_not_found() {
        let store = TaskStore::new(10, 3600);
        assert!(matches!(
            store.cancel_task("nonexistent"),
            Err(CancelError::NotFound)
        ));
    }

    #[test]
    fn test_fail_task() {
        let store = TaskStore::new(10, 3600);
        let task = store
            .create_task(None, None, test_message("fail me"))
            .unwrap();

        store.fail_task(&task.id, "something went wrong");
        let t = store.get_task(&task.id, None).unwrap();
        assert_eq!(t.status.state, TaskState::Failed);
        assert_eq!(t.history.len(), 2);
    }

    #[test]
    fn test_custom_task_id() {
        let store = TaskStore::new(10, 3600);
        let task = store
            .create_task(
                Some("my-custom-id".into()),
                Some("session-1".into()),
                test_message("hello"),
            )
            .unwrap();
        assert_eq!(task.id, "my-custom-id");
        assert_eq!(task.session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn test_history_truncation() {
        let store = TaskStore::new(10, 3600);
        let task = store
            .create_task(None, None, test_message("msg1"))
            .unwrap();
        store.complete_task(&task.id, "reply1");

        let full = store.get_task(&task.id, None).unwrap();
        assert_eq!(full.history.len(), 2);

        let truncated = store.get_task(&task.id, Some(1)).unwrap();
        assert_eq!(truncated.history.len(), 1);
    }
}
