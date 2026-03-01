//! Version-independent in-memory A2A task store with capacity limits and TTL eviction.
//!
//! This store holds tasks in a version-neutral format (`StoredTask`).
//! Each version module (v0, v1) converts `StoredTask` into its own wire format.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

// ── Version-independent task state ──────────────────────────────

/// Internal task state — superset of all version-specific states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Canceled,
    Failed,
    /// v1.0.0-rc addition
    Rejected,
    /// v1.0.0-rc addition
    AuthRequired,
    /// v1.0.0-rc addition
    Unknown,
}

/// A stored history entry (version-independent).
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub is_agent: bool,
    pub text: String,
}

/// A stored artifact (version-independent).
#[derive(Debug, Clone)]
pub struct StoredArtifact {
    pub name: String,
    pub parts: Vec<String>,
}

/// Version-independent stored task.
#[derive(Debug, Clone)]
pub struct StoredTask {
    pub id: String,
    pub session_id: Option<String>,
    pub state: TaskState,
    pub status_message: Option<String>,
    pub artifacts: Vec<StoredArtifact>,
    pub history: Vec<HistoryEntry>,
    pub created_at: SystemTime,
    pub last_modified: SystemTime,
}

// ── Task Store ──────────────────────────────────────────────────

/// Thread-safe in-memory task store.
#[derive(Clone)]
pub struct TaskStore {
    inner: Arc<Mutex<StoreInner>>,
}

struct StoreInner {
    tasks: HashMap<String, StoredTask>,
    /// Insertion-ordered task IDs for cursor-based pagination.
    task_order: Vec<String>,
    max_tasks: usize,
    task_ttl: Duration,
}

impl TaskStore {
    /// Create a new task store with the given capacity and TTL.
    pub fn new(max_tasks: usize, task_ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StoreInner {
                tasks: HashMap::new(),
                task_order: Vec::new(),
                max_tasks,
                task_ttl: Duration::from_secs(task_ttl_secs),
            })),
        }
    }

    /// Create a new task from user text. Returns the task or `None` if at capacity.
    pub fn create_task(
        &self,
        id: Option<String>,
        session_id: Option<String>,
        user_text: &str,
    ) -> Option<StoredTask> {
        let mut inner = self.inner.lock();
        self.evict_expired(&mut inner);

        if inner.tasks.len() >= inner.max_tasks {
            return None;
        }

        let task_id = id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let now = SystemTime::now();

        let task = StoredTask {
            id: task_id.clone(),
            session_id,
            state: TaskState::Submitted,
            status_message: None,
            artifacts: Vec::new(),
            history: vec![HistoryEntry {
                is_agent: false,
                text: user_text.to_string(),
            }],
            created_at: now,
            last_modified: now,
        };

        inner.task_order.push(task_id.clone());
        inner.tasks.insert(task_id, task.clone());
        Some(task)
    }

    /// Get a task by ID, optionally truncating history.
    pub fn get_task(&self, id: &str, history_length: Option<usize>) -> Option<StoredTask> {
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

    /// List tasks with optional session filter and cursor-based pagination.
    pub fn list_tasks(
        &self,
        session_id: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> (Vec<StoredTask>, Option<String>) {
        let inner = self.inner.lock();

        // Find start position from cursor (cursor = ID of next item to return)
        let start_idx = if let Some(cursor_id) = cursor {
            inner
                .task_order
                .iter()
                .position(|id| id == cursor_id)
                .unwrap_or(0)
        } else {
            0
        };

        let mut tasks = Vec::new();
        let mut next_cursor = None;

        for id in inner.task_order.iter().skip(start_idx) {
            if tasks.len() >= limit {
                next_cursor = Some(id.clone());
                break;
            }
            if let Some(task) = inner.tasks.get(id) {
                if let Some(sid) = session_id {
                    if task.session_id.as_deref() != Some(sid) {
                        continue;
                    }
                }
                tasks.push(task.clone());
            }
        }

        (tasks, next_cursor)
    }

    /// Update task state to Working.
    pub fn mark_working(&self, id: &str) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(id) {
            task.state = TaskState::Working;
            task.last_modified = SystemTime::now();
        }
    }

    /// Complete a task with the agent's response.
    pub fn complete_task(&self, id: &str, response_text: &str) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(id) {
            task.history.push(HistoryEntry {
                is_agent: true,
                text: response_text.to_string(),
            });
            task.artifacts.push(StoredArtifact {
                name: "response".into(),
                parts: vec![response_text.to_string()],
            });
            task.state = TaskState::Completed;
            task.status_message = Some(response_text.to_string());
            task.last_modified = SystemTime::now();
        }
    }

    /// Fail a task with an error message.
    pub fn fail_task(&self, id: &str, error: &str) {
        let mut inner = self.inner.lock();
        if let Some(task) = inner.tasks.get_mut(id) {
            task.history.push(HistoryEntry {
                is_agent: true,
                text: error.to_string(),
            });
            task.state = TaskState::Failed;
            task.status_message = Some(error.to_string());
            task.last_modified = SystemTime::now();
        }
    }

    /// Cancel a task. Returns the task or an error.
    pub fn cancel_task(&self, id: &str) -> Result<StoredTask, CancelError> {
        let mut inner = self.inner.lock();
        let task = inner.tasks.get_mut(id).ok_or(CancelError::NotFound)?;

        match task.state {
            TaskState::Completed | TaskState::Canceled | TaskState::Failed => {
                Err(CancelError::NotCancelable(task.state))
            }
            _ => {
                task.state = TaskState::Canceled;
                task.status_message = Some("Task canceled by client".into());
                task.last_modified = SystemTime::now();
                Ok(task.clone())
            }
        }
    }

    /// Evict tasks that have exceeded their TTL.
    fn evict_expired(&self, inner: &mut StoreInner) {
        let now = SystemTime::now();
        let expired: Vec<String> = inner
            .tasks
            .iter()
            .filter(|(_, task)| {
                now.duration_since(task.created_at)
                    .map(|age| age >= inner.task_ttl)
                    .unwrap_or(false)
            })
            .map(|(id, _)| id.clone())
            .collect();

        for id in &expired {
            inner.tasks.remove(id);
        }
        inner.task_order.retain(|id| !expired.contains(id));
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

    #[test]
    fn test_create_and_get_task() {
        let store = TaskStore::new(10, 3600);
        let task = store.create_task(None, None, "hello").unwrap();
        assert_eq!(task.state, TaskState::Submitted);
        assert_eq!(task.history.len(), 1);

        let fetched = store.get_task(&task.id, None).unwrap();
        assert_eq!(fetched.id, task.id);
    }

    #[test]
    fn test_task_lifecycle() {
        let store = TaskStore::new(10, 3600);
        let task = store.create_task(None, None, "hello").unwrap();

        store.mark_working(&task.id);
        let t = store.get_task(&task.id, None).unwrap();
        assert_eq!(t.state, TaskState::Working);

        store.complete_task(&task.id, "world");
        let t = store.get_task(&task.id, None).unwrap();
        assert_eq!(t.state, TaskState::Completed);
        assert_eq!(t.history.len(), 2);
        assert_eq!(t.artifacts.len(), 1);
    }

    #[test]
    fn test_capacity_limit() {
        let store = TaskStore::new(2, 3600);
        store.create_task(None, None, "one").unwrap();
        store.create_task(None, None, "two").unwrap();
        assert!(store.create_task(None, None, "three").is_none());
    }

    #[test]
    fn test_ttl_eviction() {
        let store = TaskStore::new(10, 0);
        store.create_task(None, None, "ephemeral").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let task = store.create_task(None, None, "new").unwrap();
        assert!(store.get_task(&task.id, None).is_some());
    }

    #[test]
    fn test_cancel_task() {
        let store = TaskStore::new(10, 3600);
        let task = store.create_task(None, None, "cancel me").unwrap();

        store.mark_working(&task.id);
        let canceled = store.cancel_task(&task.id).unwrap();
        assert_eq!(canceled.state, TaskState::Canceled);

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
        let task = store.create_task(None, None, "fail me").unwrap();

        store.fail_task(&task.id, "something went wrong");
        let t = store.get_task(&task.id, None).unwrap();
        assert_eq!(t.state, TaskState::Failed);
        assert_eq!(t.history.len(), 2);
    }

    #[test]
    fn test_custom_task_id() {
        let store = TaskStore::new(10, 3600);
        let task = store
            .create_task(Some("my-custom-id".into()), Some("session-1".into()), "hello")
            .unwrap();
        assert_eq!(task.id, "my-custom-id");
        assert_eq!(task.session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn test_history_truncation() {
        let store = TaskStore::new(10, 3600);
        let task = store.create_task(None, None, "msg1").unwrap();
        store.complete_task(&task.id, "reply1");

        let full = store.get_task(&task.id, None).unwrap();
        assert_eq!(full.history.len(), 2);

        let truncated = store.get_task(&task.id, Some(1)).unwrap();
        assert_eq!(truncated.history.len(), 1);
    }

    #[test]
    fn test_list_tasks_basic() {
        let store = TaskStore::new(10, 3600);
        store
            .create_task(Some("t1".into()), Some("s1".into()), "one")
            .unwrap();
        store
            .create_task(Some("t2".into()), Some("s1".into()), "two")
            .unwrap();
        store
            .create_task(Some("t3".into()), Some("s2".into()), "three")
            .unwrap();

        let (tasks, cursor) = store.list_tasks(None, None, 10);
        assert_eq!(tasks.len(), 3);
        assert!(cursor.is_none());

        let (tasks, _) = store.list_tasks(Some("s1"), None, 10);
        assert_eq!(tasks.len(), 2);

        let (tasks, cursor) = store.list_tasks(None, None, 2);
        assert_eq!(tasks.len(), 2);
        assert!(cursor.is_some());

        let (tasks2, cursor2) = store.list_tasks(None, cursor.as_deref(), 10);
        assert_eq!(tasks2.len(), 1);
        assert!(cursor2.is_none());
    }

    #[test]
    fn test_last_modified_updates() {
        let store = TaskStore::new(10, 3600);
        let task = store.create_task(None, None, "hello").unwrap();
        let created = task.last_modified;

        std::thread::sleep(std::time::Duration::from_millis(10));
        store.mark_working(&task.id);

        let t = store.get_task(&task.id, None).unwrap();
        assert!(t.last_modified > created);
    }
}
