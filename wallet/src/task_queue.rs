use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum TaskQueueError {
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("duplicate task: {0}")]
    DuplicateTask(String),
    #[error("queue not found: {0}")]
    QueueNotFound(String),
    #[error("max retries exceeded: {0}")]
    MaxRetriesExceeded(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority2 {
    Critical,
    High,
    #[default]
    Normal,
    Low,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus3 {
    #[default]
    Queued,
    Running,
    Completed,
    Failed,
    DeadLetter,
    Cancelled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskType2 {
    TxSubmit,
    BalanceRefresh,
    EnergyCheck,
    Backup,
    #[default]
    Sync,
    Notification,
    Custom(String),
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueTask {
    pub id: String,
    pub task_type: TaskType2,
    pub priority: TaskPriority2,
    pub status: TaskStatus3,
    pub payload: HashMap<String, String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub progress_pct: f64,
}

impl Default for QueueTask {
    fn default() -> Self {
        Self {
            id: String::new(),
            task_type: TaskType2::default(),
            priority: TaskPriority2::default(),
            status: TaskStatus3::default(),
            payload: HashMap::new(),
            result: None,
            error: None,
            retry_count: 0,
            max_retries: 3,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
            progress_pct: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterEntry {
    pub task: QueueTask,
    pub moved_at: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueueStats2 {
    pub total_tasks: usize,
    pub queued: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub dead_letter: usize,
    pub cancelled: usize,
    pub total_retries: u32,
    pub avg_completion_time_ms: f64,
}

// ---------------------------------------------------------------------------
// TaskQueue
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskQueue {
    pub tasks: HashMap<String, QueueTask>,
    pub dead_letter: Vec<DeadLetterEntry>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a task. Fails if a task with the same id already exists.
    pub fn enqueue(&mut self, task: QueueTask) -> Result<(), TaskQueueError> {
        if self.tasks.contains_key(&task.id) {
            return Err(TaskQueueError::DuplicateTask(task.id.clone()));
        }
        self.tasks.insert(task.id.clone(), task);
        Ok(())
    }

    /// Dequeue the highest-priority Queued task and set it to Running.
    pub fn dequeue(&mut self) -> Option<QueueTask> {
        let best_id = self
            .tasks
            .values()
            .filter(|t| t.status == TaskStatus3::Queued)
            .min_by_key(|t| t.priority.clone())
            .map(|t| t.id.clone());

        if let Some(id) = best_id {
            if let Some(task) = self.tasks.get_mut(&id) {
                task.status = TaskStatus3::Running;
                task.started_at = Some(Utc::now().to_rfc3339());
                return Some(task.clone());
            }
        }
        None
    }

    /// Mark a task as Completed with a result string.
    pub fn complete_task(&mut self, id: &str, result: &str) -> Result<(), TaskQueueError> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| TaskQueueError::TaskNotFound(id.to_string()))?;
        task.status = TaskStatus3::Completed;
        task.result = Some(result.to_string());
        task.completed_at = Some(Utc::now().to_rfc3339());
        task.progress_pct = 100.0;
        Ok(())
    }

    /// Fail a task. If retries remain, re-queue it; otherwise move to dead letter.
    pub fn fail_task(&mut self, id: &str, error: &str) -> Result<(), TaskQueueError> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| TaskQueueError::TaskNotFound(id.to_string()))?;
        task.retry_count += 1;
        task.error = Some(error.to_string());

        if task.retry_count >= task.max_retries {
            let mut moved_task = task.clone();
            moved_task.status = TaskStatus3::DeadLetter;
            self.dead_letter.push(DeadLetterEntry {
                task: moved_task,
                moved_at: Utc::now().to_rfc3339(),
                reason: format!("max retries exceeded: {}", error),
            });
            self.tasks.remove(id);
        } else {
            task.status = TaskStatus3::Queued;
        }
        Ok(())
    }

    /// Cancel a task.
    pub fn cancel_task(&mut self, id: &str) -> Result<(), TaskQueueError> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| TaskQueueError::TaskNotFound(id.to_string()))?;
        task.status = TaskStatus3::Cancelled;
        Ok(())
    }

    /// Update progress percentage of a task.
    pub fn update_progress(&mut self, id: &str, pct: f64) -> Result<(), TaskQueueError> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| TaskQueueError::TaskNotFound(id.to_string()))?;
        task.progress_pct = pct;
        Ok(())
    }

    /// Move a task from dead letter back to the active queue, resetting retries.
    pub fn retry_dead_letter(&mut self, id: &str) -> Result<(), TaskQueueError> {
        let pos = self
            .dead_letter
            .iter()
            .position(|e| e.task.id == id)
            .ok_or_else(|| TaskQueueError::TaskNotFound(id.to_string()))?;
        let mut entry = self.dead_letter.remove(pos);
        entry.task.status = TaskStatus3::Queued;
        entry.task.retry_count = 0;
        entry.task.error = None;
        self.tasks.insert(entry.task.id.clone(), entry.task);
        Ok(())
    }

    pub fn get_task(&self, id: &str) -> Option<&QueueTask> {
        self.tasks.get(id)
    }

    pub fn tasks_by_status(&self, status: &TaskStatus3) -> Vec<&QueueTask> {
        self.tasks
            .values()
            .filter(|t| t.status == *status)
            .collect()
    }

    pub fn tasks_by_priority(&self, priority: &TaskPriority2) -> Vec<&QueueTask> {
        self.tasks
            .values()
            .filter(|t| t.priority == *priority)
            .collect()
    }

    pub fn running_tasks(&self) -> Vec<&QueueTask> {
        self.tasks_by_status(&TaskStatus3::Running)
    }

    pub fn dead_letter_tasks(&self) -> Vec<&DeadLetterEntry> {
        self.dead_letter.iter().collect()
    }

    /// Remove all completed tasks; return how many were removed.
    pub fn purge_completed(&mut self) -> usize {
        let before = self.tasks.len();
        self.tasks.retain(|_, t| t.status != TaskStatus3::Completed);
        before - self.tasks.len()
    }

    /// Number of tasks currently in Queued status.
    pub fn queue_depth(&self) -> usize {
        self.tasks
            .values()
            .filter(|t| t.status == TaskStatus3::Queued)
            .count()
    }

    #[allow(clippy::field_reassign_with_default)]
    pub fn stats(&self) -> QueueStats2 {
        let mut s = QueueStats2::default();
        s.total_tasks = self.tasks.len() + self.dead_letter.len();
        for t in self.tasks.values() {
            match t.status {
                TaskStatus3::Queued => s.queued += 1,
                TaskStatus3::Running => s.running += 1,
                TaskStatus3::Completed => s.completed += 1,
                TaskStatus3::Failed => s.failed += 1,
                TaskStatus3::DeadLetter => s.dead_letter += 1,
                TaskStatus3::Cancelled => s.cancelled += 1,
            }
            s.total_retries += t.retry_count;
        }
        s.dead_letter += self.dead_letter.len();
        for e in &self.dead_letter {
            s.total_retries += e.task.retry_count;
        }

        // avg completion time — we don't track real durations here, report 0.0
        s.avg_completion_time_ms = 0.0;
        s
    }

    pub fn save(&self, path: &Path) -> Result<(), TaskQueueError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, TaskQueueError> {
        let data = std::fs::read_to_string(path)?;
        let queue: Self = serde_json::from_str(&data)?;
        Ok(queue)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(id: &str, priority: TaskPriority2, task_type: TaskType2) -> QueueTask {
        QueueTask {
            id: id.to_string(),
            task_type,
            priority,
            status: TaskStatus3::Queued,
            max_retries: 3,
            ..Default::default()
        }
    }

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_tq_{}_{}.json",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn test_enqueue_basic() {
        let mut q = TaskQueue::new();
        let t = make_task("t1", TaskPriority2::Normal, TaskType2::TxSubmit);
        assert!(q.enqueue(t).is_ok());
        assert_eq!(q.tasks.len(), 1);
    }

    #[test]
    fn test_enqueue_duplicate() {
        let mut q = TaskQueue::new();
        let t1 = make_task("t1", TaskPriority2::Normal, TaskType2::TxSubmit);
        let t2 = make_task("t1", TaskPriority2::High, TaskType2::Sync);
        q.enqueue(t1).unwrap();
        let err = q.enqueue(t2).unwrap_err();
        assert!(matches!(err, TaskQueueError::DuplicateTask(_)));
    }

    #[test]
    fn test_dequeue_basic() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("t1", TaskPriority2::Normal, TaskType2::Sync))
            .unwrap();
        let dequeued = q.dequeue().unwrap();
        assert_eq!(dequeued.id, "t1");
        assert_eq!(dequeued.status, TaskStatus3::Running);
    }

    #[test]
    fn test_dequeue_priority_order() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("low", TaskPriority2::Low, TaskType2::Backup))
            .unwrap();
        q.enqueue(make_task("normal", TaskPriority2::Normal, TaskType2::Sync))
            .unwrap();
        q.enqueue(make_task(
            "critical",
            TaskPriority2::Critical,
            TaskType2::TxSubmit,
        ))
        .unwrap();
        let first = q.dequeue().unwrap();
        assert_eq!(first.id, "critical");
        let second = q.dequeue().unwrap();
        assert_eq!(second.id, "normal");
    }

    #[test]
    fn test_dequeue_empty() {
        let mut q = TaskQueue::new();
        assert!(q.dequeue().is_none());
    }

    #[test]
    fn test_complete_task() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("t1", TaskPriority2::Normal, TaskType2::TxSubmit))
            .unwrap();
        q.dequeue();
        q.complete_task("t1", "tx_hash_abc").unwrap();
        let t = q.get_task("t1").unwrap();
        assert_eq!(t.status, TaskStatus3::Completed);
        assert_eq!(t.result.as_deref(), Some("tx_hash_abc"));
        assert!(t.completed_at.is_some());
        assert!((t.progress_pct - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_fail_task_with_retry() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("t1", TaskPriority2::Normal, TaskType2::Sync))
            .unwrap();
        q.dequeue();
        q.fail_task("t1", "timeout").unwrap();
        let t = q.get_task("t1").unwrap();
        assert_eq!(t.status, TaskStatus3::Queued);
        assert_eq!(t.retry_count, 1);
    }

    #[test]
    fn test_fail_task_to_dead_letter() {
        let mut q = TaskQueue::new();
        let mut task = make_task("t1", TaskPriority2::Normal, TaskType2::Sync);
        task.max_retries = 2;
        q.enqueue(task).unwrap();
        q.dequeue();
        q.fail_task("t1", "err1").unwrap(); // retry_count=1, re-queued
        q.dequeue();
        q.fail_task("t1", "err2").unwrap(); // retry_count=2 >= max_retries=2 -> dead letter
        assert!(q.get_task("t1").is_none());
        assert_eq!(q.dead_letter.len(), 1);
        assert_eq!(q.dead_letter[0].task.id, "t1");
    }

    #[test]
    fn test_cancel_task() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("t1", TaskPriority2::Normal, TaskType2::Backup))
            .unwrap();
        q.cancel_task("t1").unwrap();
        assert_eq!(q.get_task("t1").unwrap().status, TaskStatus3::Cancelled);
    }

    #[test]
    fn test_update_progress() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("t1", TaskPriority2::Normal, TaskType2::Sync))
            .unwrap();
        q.dequeue();
        q.update_progress("t1", 55.5).unwrap();
        assert!((q.get_task("t1").unwrap().progress_pct - 55.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_retry_dead_letter() {
        let mut q = TaskQueue::new();
        let mut task = make_task("t1", TaskPriority2::High, TaskType2::TxSubmit);
        task.max_retries = 1;
        q.enqueue(task).unwrap();
        q.dequeue();
        q.fail_task("t1", "network").unwrap(); // goes to dead letter
        assert!(q.get_task("t1").is_none());
        q.retry_dead_letter("t1").unwrap();
        let t = q.get_task("t1").unwrap();
        assert_eq!(t.status, TaskStatus3::Queued);
        assert_eq!(t.retry_count, 0);
        assert!(q.dead_letter.is_empty());
    }

    #[test]
    fn test_tasks_by_status() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("a", TaskPriority2::Normal, TaskType2::Sync))
            .unwrap();
        q.enqueue(make_task("b", TaskPriority2::Normal, TaskType2::Backup))
            .unwrap();
        q.dequeue(); // one becomes Running
        let queued = q.tasks_by_status(&TaskStatus3::Queued);
        assert_eq!(queued.len(), 1);
        let running = q.tasks_by_status(&TaskStatus3::Running);
        assert_eq!(running.len(), 1);
    }

    #[test]
    fn test_tasks_by_priority() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("a", TaskPriority2::High, TaskType2::TxSubmit))
            .unwrap();
        q.enqueue(make_task("b", TaskPriority2::High, TaskType2::Sync))
            .unwrap();
        q.enqueue(make_task("c", TaskPriority2::Low, TaskType2::Backup))
            .unwrap();
        let high = q.tasks_by_priority(&TaskPriority2::High);
        assert_eq!(high.len(), 2);
    }

    #[test]
    fn test_running_tasks() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("a", TaskPriority2::Normal, TaskType2::Sync))
            .unwrap();
        q.enqueue(make_task("b", TaskPriority2::High, TaskType2::TxSubmit))
            .unwrap();
        q.dequeue();
        q.dequeue();
        assert_eq!(q.running_tasks().len(), 2);
    }

    #[test]
    fn test_dead_letter_listing() {
        let mut q = TaskQueue::new();
        let mut task = make_task("t1", TaskPriority2::Normal, TaskType2::Sync);
        task.max_retries = 1;
        q.enqueue(task).unwrap();
        q.dequeue();
        q.fail_task("t1", "boom").unwrap();
        let dl = q.dead_letter_tasks();
        assert_eq!(dl.len(), 1);
        assert_eq!(dl[0].task.id, "t1");
    }

    #[test]
    fn test_purge_completed() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("a", TaskPriority2::Normal, TaskType2::Sync))
            .unwrap();
        q.enqueue(make_task("b", TaskPriority2::Normal, TaskType2::Backup))
            .unwrap();
        q.enqueue(make_task("c", TaskPriority2::Normal, TaskType2::TxSubmit))
            .unwrap();
        q.dequeue();
        q.dequeue();
        q.complete_task("a", "ok").unwrap();
        q.complete_task("b", "ok").unwrap();
        let purged = q.purge_completed();
        assert_eq!(purged, 2);
        assert_eq!(q.tasks.len(), 1);
    }

    #[test]
    fn test_queue_depth() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("a", TaskPriority2::Normal, TaskType2::Sync))
            .unwrap();
        q.enqueue(make_task("b", TaskPriority2::High, TaskType2::TxSubmit))
            .unwrap();
        assert_eq!(q.queue_depth(), 2);
        q.dequeue();
        assert_eq!(q.queue_depth(), 1);
    }

    #[test]
    fn test_stats() {
        let mut q = TaskQueue::new();
        q.enqueue(make_task("a", TaskPriority2::Normal, TaskType2::Sync))
            .unwrap();
        q.enqueue(make_task("b", TaskPriority2::High, TaskType2::TxSubmit))
            .unwrap();
        q.dequeue();
        q.complete_task("b", "done").unwrap();
        let s = q.stats();
        assert_eq!(s.total_tasks, 2);
        assert_eq!(s.queued, 1);
        assert_eq!(s.completed, 1);
    }

    #[test]
    fn test_persistence_save_load() {
        let path = test_path("persist");
        let mut q = TaskQueue::new();
        q.enqueue(make_task(
            "t1",
            TaskPriority2::Critical,
            TaskType2::TxSubmit,
        ))
        .unwrap();
        q.save(&path).unwrap();

        let loaded = TaskQueue::load(&path).unwrap();
        assert_eq!(loaded.tasks.len(), 1);
        assert!(loaded.get_task("t1").is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nonexistent");
        let _ = std::fs::remove_file(&path);
        let q = TaskQueue::load_or_default(&path);
        assert!(q.tasks.is_empty());
    }

    #[test]
    fn test_task_not_found_complete() {
        let mut q = TaskQueue::new();
        let err = q.complete_task("nope", "ok").unwrap_err();
        assert!(matches!(err, TaskQueueError::TaskNotFound(_)));
    }

    #[test]
    fn test_custom_task_type() {
        let mut q = TaskQueue::new();
        let task = make_task(
            "c1",
            TaskPriority2::Normal,
            TaskType2::Custom("audit".into()),
        );
        q.enqueue(task).unwrap();
        let t = q.get_task("c1").unwrap();
        assert_eq!(t.task_type, TaskType2::Custom("audit".into()));
    }
}
