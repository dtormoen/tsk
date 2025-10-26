use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Represents the execution status of a task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    /// Task is in the queue waiting to be executed
    #[serde(rename = "QUEUED")]
    Queued,
    /// Task is currently being executed
    #[serde(rename = "RUNNING")]
    Running,
    /// Task execution failed
    #[serde(rename = "FAILED")]
    Failed,
    /// Task completed successfully
    #[serde(rename = "COMPLETE")]
    Complete,
}

/// Represents a TSK task with all required fields for execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier for the task (format: YYYY-MM-DD-HHMM-{task_type}-{name})
    pub id: String,
    /// Absolute path to the repository root where the task was created
    pub repo_root: PathBuf,
    /// Human-readable name for the task
    pub name: String,
    /// Type of task (e.g., "feat", "fix", "refactor")
    pub task_type: String,
    /// Path to the instructions file containing task details
    pub instructions_file: String,
    /// AI agent to use for task execution (e.g., "claude")
    pub agent: String,
    /// Current status of the task
    pub status: TaskStatus,
    /// When the task was created
    pub created_at: DateTime<Local>,
    /// When the task started execution (if started)
    pub started_at: Option<DateTime<Utc>>,
    /// When the task completed (if completed)
    pub completed_at: Option<DateTime<Utc>>,
    /// Git branch name for this task (format: tsk/{task-id})
    pub branch_name: String,
    /// Error message if task failed
    pub error_message: Option<String>,
    /// Git commit SHA from which the task was created
    pub source_commit: String,
    /// Stack for Docker image selection (e.g., "rust", "python", "default")
    #[serde(alias = "tech_stack")]
    pub stack: String,
    /// Project name for Docker image selection (defaults to "default")
    pub project: String,
    /// Path to the copied repository for this task
    pub copied_repo_path: PathBuf,
    /// Whether this task should run in interactive mode
    #[serde(default)]
    pub is_interactive: bool,
}

impl Task {
    /// Creates a new Task with all required fields
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        repo_root: PathBuf,
        name: String,
        task_type: String,
        instructions_file: String,
        agent: String,
        branch_name: String,
        source_commit: String,
        stack: String,
        project: String,
        created_at: DateTime<Local>,
        copied_repo_path: PathBuf,
        is_interactive: bool,
    ) -> Self {
        Self {
            id,
            repo_root,
            name,
            task_type,
            instructions_file,
            agent,
            status: TaskStatus::Queued,
            created_at,
            started_at: None,
            completed_at: None,
            branch_name,
            error_message: None,
            source_commit,
            stack,
            project,
            copied_repo_path,
            is_interactive,
        }
    }
}

// TaskBuilder has been moved to task_builder.rs
// Re-export it for backward compatibility
pub use crate::task_builder::TaskBuilder;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_serialization() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Queued).unwrap(),
            "\"QUEUED\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Running).unwrap(),
            "\"RUNNING\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Failed).unwrap(),
            "\"FAILED\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Complete).unwrap(),
            "\"COMPLETE\""
        );
    }

    #[test]
    fn test_task_creation() {
        let task = Task::new(
            "test-id".to_string(),
            PathBuf::from("/test"),
            "test-task".to_string(),
            "feat".to_string(),
            "instructions.md".to_string(),
            "claude".to_string(),
            "tsk/feat/test-task/test-id".to_string(),
            "abc123".to_string(),
            "rust".to_string(),
            "test-project".to_string(),
            chrono::Local::now(),
            PathBuf::from("/test/copied"),
            false,
        );

        assert_eq!(task.id, "test-id");
        assert_eq!(task.name, "test-task");
        assert_eq!(task.task_type, "feat");
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.started_at.is_none());
        assert!(task.completed_at.is_none());
        assert!(task.error_message.is_none());
        assert!(!task.is_interactive);
    }
}
