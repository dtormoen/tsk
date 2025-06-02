use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    #[serde(rename = "QUEUED")]
    Queued,
    #[serde(rename = "RUNNING")]
    Running,
    #[serde(rename = "FAILED")]
    Failed,
    #[serde(rename = "COMPLETE")]
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub task_type: String,
    pub description: Option<String>,
    pub instructions_file: Option<String>,
    pub agent: Option<String>,
    pub timeout: u32,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub branch_name: Option<String>,
    pub error_message: Option<String>,
}

impl Task {
    pub fn new(
        name: String,
        task_type: String,
        description: Option<String>,
        instructions_file: Option<String>,
        agent: Option<String>,
        timeout: u32,
    ) -> Self {
        let timestamp = Utc::now();
        let id = format!("{}-{}", timestamp.format("%Y-%m-%d-%H%M"), name);

        Self {
            id,
            name,
            task_type,
            description,
            instructions_file,
            agent,
            timeout,
            status: TaskStatus::Queued,
            created_at: timestamp,
            started_at: None,
            completed_at: None,
            branch_name: None,
            error_message: None,
        }
    }
}

// Trait for task storage abstraction
#[async_trait::async_trait]
pub trait TaskStorage: Send + Sync {
    async fn add_task(&self, task: Task) -> Result<(), Box<dyn std::error::Error>>;
    #[allow(dead_code)] // Will be used in future functionality (e.g., task details view)
    async fn get_task(&self, id: &str) -> Result<Option<Task>, Box<dyn std::error::Error>>;
    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error>>;
    async fn update_task(&self, task: Task) -> Result<(), Box<dyn std::error::Error>>;
    async fn delete_task(&self, id: &str) -> Result<(), Box<dyn std::error::Error>>;
    async fn delete_tasks_by_status(
        &self,
        statuses: Vec<TaskStatus>,
    ) -> Result<usize, Box<dyn std::error::Error>>;
}

// JSON file-based implementation
pub struct JsonTaskStorage {
    file_path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl JsonTaskStorage {
    pub fn new(base_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let file_path = base_path.join("tasks.json");

        // Ensure the directory exists
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        Ok(Self {
            file_path,
            lock: Arc::new(Mutex::new(())),
        })
    }

    async fn read_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error>> {
        if !self.file_path.exists() {
            return Ok(Vec::new());
        }

        let contents = tokio::fs::read_to_string(&self.file_path).await?;
        let tasks: Vec<Task> = serde_json::from_str(&contents)?;
        Ok(tasks)
    }

    async fn write_tasks(&self, tasks: &[Task]) -> Result<(), Box<dyn std::error::Error>> {
        let contents = serde_json::to_string_pretty(tasks)?;
        tokio::fs::write(&self.file_path, contents).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl TaskStorage for JsonTaskStorage {
    async fn add_task(&self, task: Task) -> Result<(), Box<dyn std::error::Error>> {
        let _lock = self.lock.lock().await;

        let mut tasks = self.read_tasks().await?;
        tasks.push(task);
        self.write_tasks(&tasks).await?;

        Ok(())
    }

    async fn get_task(&self, id: &str) -> Result<Option<Task>, Box<dyn std::error::Error>> {
        let tasks = self.read_tasks().await?;
        Ok(tasks.into_iter().find(|t| t.id == id))
    }

    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error>> {
        self.read_tasks().await
    }

    async fn update_task(&self, task: Task) -> Result<(), Box<dyn std::error::Error>> {
        let _lock = self.lock.lock().await;

        let mut tasks = self.read_tasks().await?;
        if let Some(index) = tasks.iter().position(|t| t.id == task.id) {
            tasks[index] = task;
            self.write_tasks(&tasks).await?;
            Ok(())
        } else {
            Err("Task not found".into())
        }
    }

    async fn delete_task(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let _lock = self.lock.lock().await;

        let mut tasks = self.read_tasks().await?;
        if let Some(index) = tasks.iter().position(|t| t.id == id) {
            tasks.remove(index);
            self.write_tasks(&tasks).await?;
            Ok(())
        } else {
            Err("Task not found".into())
        }
    }

    async fn delete_tasks_by_status(
        &self,
        statuses: Vec<TaskStatus>,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let _lock = self.lock.lock().await;

        let mut tasks = self.read_tasks().await?;
        let original_count = tasks.len();
        tasks.retain(|t| !statuses.contains(&t.status));
        let deleted_count = original_count - tasks.len();
        self.write_tasks(&tasks).await?;
        Ok(deleted_count)
    }
}

// Factory function for getting task storage
pub fn get_task_storage() -> Result<Box<dyn TaskStorage>, Box<dyn std::error::Error>> {
    let storage = JsonTaskStorage::new(Path::new(".tsk"))?;
    Ok(Box::new(storage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_json_task_storage() {
        let temp_dir = TempDir::new().unwrap();
        let storage = JsonTaskStorage::new(temp_dir.path()).unwrap();

        // Test adding a task
        let task = Task::new(
            "test-task".to_string(),
            "feature".to_string(),
            Some("Test description".to_string()),
            None,
            None,
            30,
        );

        storage.add_task(task.clone()).await.unwrap();

        // Test getting a task
        let retrieved = storage.get_task(&task.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test-task");

        // Test listing tasks
        let tasks = storage.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);

        // Test updating a task
        let mut updated_task = task.clone();
        updated_task.status = TaskStatus::Running;
        storage.update_task(updated_task).await.unwrap();

        let retrieved = storage.get_task(&task.id).await.unwrap().unwrap();
        assert_eq!(retrieved.status, TaskStatus::Running);

        // Test deleting a task
        storage.delete_task(&task.id).await.unwrap();
        let retrieved = storage.get_task(&task.id).await.unwrap();
        assert!(retrieved.is_none());

        // Test deleting tasks by status
        let task1 = Task::new(
            "task1".to_string(),
            "feature".to_string(),
            Some("Task 1".to_string()),
            None,
            None,
            30,
        );
        let mut task2 = Task::new(
            "task2".to_string(),
            "bug-fix".to_string(),
            Some("Task 2".to_string()),
            None,
            None,
            30,
        );
        task2.status = TaskStatus::Complete;
        let mut task3 = Task::new(
            "task3".to_string(),
            "refactor".to_string(),
            Some("Task 3".to_string()),
            None,
            None,
            30,
        );
        task3.status = TaskStatus::Failed;

        storage.add_task(task1.clone()).await.unwrap();
        storage.add_task(task2.clone()).await.unwrap();
        storage.add_task(task3.clone()).await.unwrap();

        // Delete completed and failed tasks
        let deleted_count = storage
            .delete_tasks_by_status(vec![TaskStatus::Complete, TaskStatus::Failed])
            .await
            .unwrap();
        assert_eq!(deleted_count, 2);

        // Verify only queued task remains
        let remaining_tasks = storage.list_tasks().await.unwrap();
        assert_eq!(remaining_tasks.len(), 1);
        assert_eq!(remaining_tasks[0].status, TaskStatus::Queued);
    }
}
