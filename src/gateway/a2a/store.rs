//! In-memory A2A task store with capacity limits and TTL eviction.

use super::types::*;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// In-memory task store with capacity limits and TTL eviction.
#[derive(Debug, Clone)]
pub struct A2ATaskStore {
    inner: Arc<Mutex<TaskStoreInner>>,
    max_tasks: usize,
    task_ttl: Duration,
}

#[derive(Debug)]
struct TaskStoreInner {
    tasks: HashMap<String, TaskRecord>,
}

impl A2ATaskStore {
    pub fn new(max_tasks: usize, task_ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(TaskStoreInner {
                tasks: HashMap::new(),
            })),
            max_tasks: max_tasks.max(1),
            task_ttl: Duration::from_secs(task_ttl_secs),
        }
    }

    /// Evict completed/failed/canceled tasks past TTL, then oldest if still over capacity.
    fn evict_if_needed(inner: &mut TaskStoreInner, max_tasks: usize, ttl: Duration) {
        let now = Instant::now();

        // First pass: remove expired terminal tasks
        inner.tasks.retain(|_, record| {
            if let Some(completed_at) = record.completed_at {
                now.duration_since(completed_at) < ttl
            } else {
                true
            }
        });

        // Second pass: if still over capacity, evict oldest completed task
        while inner.tasks.len() >= max_tasks {
            let oldest_terminal = inner
                .tasks
                .iter()
                .filter(|(_, r)| r.completed_at.is_some())
                .min_by_key(|(_, r)| r.completed_at.unwrap())
                .map(|(k, _)| k.clone());

            if let Some(key) = oldest_terminal {
                inner.tasks.remove(&key);
            } else {
                break;
            }
        }
    }

    /// Create a new task in `submitted` state.
    pub fn create_task(&self, task_id: &str, initial_message: Message) -> A2ATask {
        let mut inner = self.inner.lock();
        Self::evict_if_needed(&mut inner, self.max_tasks, self.task_ttl);

        let task = A2ATask {
            id: task_id.to_string(),
            status: TaskStatusInfo {
                state: TaskState::Submitted,
                message: None,
            },
            artifacts: Vec::new(),
            history: vec![initial_message],
        };

        inner.tasks.insert(
            task_id.to_string(),
            TaskRecord {
                task: task.clone(),
                created_at: Instant::now(),
                completed_at: None,
            },
        );

        task
    }

    /// Transition a task to `working` state.
    pub fn set_working(&self, task_id: &str) {
        let mut inner = self.inner.lock();
        if let Some(record) = inner.tasks.get_mut(task_id) {
            record.task.status.state = TaskState::Working;
        }
    }

    /// Complete a task with response text and message.
    pub fn set_completed(
        &self,
        task_id: &str,
        response_text: String,
        response_message: Message,
    ) {
        let mut inner = self.inner.lock();
        if let Some(record) = inner.tasks.get_mut(task_id) {
            record.task.status = TaskStatusInfo {
                state: TaskState::Completed,
                message: Some(response_message),
            };
            record.task.artifacts.push(Artifact {
                parts: vec![Part::Text {
                    text: response_text,
                }],
                index: record.task.artifacts.len() as u32,
            });
            record.completed_at = Some(Instant::now());
        }
    }

    /// Fail a task with an error message.
    pub fn set_failed(&self, task_id: &str, error_msg: &str) {
        let mut inner = self.inner.lock();
        if let Some(record) = inner.tasks.get_mut(task_id) {
            record.task.status = TaskStatusInfo {
                state: TaskState::Failed,
                message: Some(Message {
                    role: "agent".into(),
                    parts: vec![Part::Text {
                        text: error_msg.to_string(),
                    }],
                }),
            };
            record.completed_at = Some(Instant::now());
        }
    }

    /// Cancel a task. Returns Ok(()) if cancelable, Err with reason if not.
    pub fn set_canceled(&self, task_id: &str) -> Result<(), &'static str> {
        let mut inner = self.inner.lock();
        let Some(record) = inner.tasks.get_mut(task_id) else {
            return Err("task not found");
        };

        match record.task.status.state {
            TaskState::Submitted | TaskState::Working => {
                record.task.status = TaskStatusInfo {
                    state: TaskState::Canceled,
                    message: None,
                };
                record.completed_at = Some(Instant::now());
                Ok(())
            }
            _ => Err("task is already in a terminal state"),
        }
    }

    /// Get a task by ID.
    pub fn get_task(&self, task_id: &str) -> Option<A2ATask> {
        let inner = self.inner.lock();
        inner.tasks.get(task_id).map(|r| r.task.clone())
    }

    /// Check if a task exists.
    pub fn task_exists(&self, task_id: &str) -> bool {
        let inner = self.inner.lock();
        inner.tasks.contains_key(task_id)
    }

    /// Add a message to task history.
    pub fn add_to_history(&self, task_id: &str, message: Message) {
        let mut inner = self.inner.lock();
        if let Some(record) = inner.tasks.get_mut(task_id) {
            record.task.history.push(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_message(role: &str, text: &str) -> Message {
        Message {
            role: role.into(),
            parts: vec![Part::Text {
                text: text.into(),
            }],
        }
    }

    #[test]
    fn create_task_returns_submitted_state() {
        let store = A2ATaskStore::new(100, 3600);
        let task = store.create_task("task-1", text_message("user", "hello"));
        assert_eq!(task.id, "task-1");
        assert_eq!(task.status.state, TaskState::Submitted);
        assert_eq!(task.history.len(), 1);
        assert!(task.artifacts.is_empty());
    }

    #[test]
    fn task_lifecycle_submitted_working_completed() {
        let store = A2ATaskStore::new(100, 3600);
        store.create_task("t1", text_message("user", "hello"));

        store.set_working("t1");
        let task = store.get_task("t1").unwrap();
        assert_eq!(task.status.state, TaskState::Working);

        store.set_completed(
            "t1",
            "response".into(),
            text_message("agent", "response"),
        );
        let task = store.get_task("t1").unwrap();
        assert_eq!(task.status.state, TaskState::Completed);
        assert_eq!(task.artifacts.len(), 1);
    }

    #[test]
    fn cancel_submitted_task_succeeds() {
        let store = A2ATaskStore::new(100, 3600);
        store.create_task("t1", text_message("user", "hello"));
        assert!(store.set_canceled("t1").is_ok());
        let task = store.get_task("t1").unwrap();
        assert_eq!(task.status.state, TaskState::Canceled);
    }

    #[test]
    fn cancel_completed_task_fails() {
        let store = A2ATaskStore::new(100, 3600);
        store.create_task("t1", text_message("user", "hello"));
        store.set_completed(
            "t1",
            "done".into(),
            text_message("agent", "done"),
        );
        assert!(store.set_canceled("t1").is_err());
    }

    #[test]
    fn eviction_removes_oldest_completed_when_at_capacity() {
        let store = A2ATaskStore::new(2, 3600);
        store.create_task("t1", text_message("user", "a"));
        store.set_completed("t1", "r1".into(), text_message("agent", "r1"));

        std::thread::sleep(std::time::Duration::from_millis(2));
        store.create_task("t2", text_message("user", "b"));
        store.set_completed("t2", "r2".into(), text_message("agent", "r2"));

        // Creating t3 should evict t1 (oldest completed)
        store.create_task("t3", text_message("user", "c"));
        assert!(store.get_task("t1").is_none());
        assert!(store.get_task("t2").is_some());
        assert!(store.get_task("t3").is_some());
    }

    #[test]
    fn ttl_eviction_removes_expired_tasks() {
        let store = A2ATaskStore::new(100, 0); // 0 second TTL
        store.create_task("t1", text_message("user", "a"));
        store.set_completed("t1", "r".into(), text_message("agent", "r"));

        std::thread::sleep(std::time::Duration::from_millis(10));

        // Creating a new task triggers eviction
        store.create_task("t2", text_message("user", "b"));
        assert!(store.get_task("t1").is_none());
    }

    #[test]
    fn add_to_history_appends_message() {
        let store = A2ATaskStore::new(100, 3600);
        store.create_task("t1", text_message("user", "hello"));
        store.add_to_history("t1", text_message("agent", "hi"));

        let task = store.get_task("t1").unwrap();
        assert_eq!(task.history.len(), 2);
    }

    #[test]
    fn get_nonexistent_task_returns_none() {
        let store = A2ATaskStore::new(100, 3600);
        assert!(store.get_task("nonexistent").is_none());
    }

    #[test]
    fn set_failed_marks_task_as_failed() {
        let store = A2ATaskStore::new(100, 3600);
        store.create_task("t1", text_message("user", "hello"));
        store.set_failed("t1", "something went wrong");
        let task = store.get_task("t1").unwrap();
        assert_eq!(task.status.state, TaskState::Failed);
        assert!(task.status.message.is_some());
    }
}
