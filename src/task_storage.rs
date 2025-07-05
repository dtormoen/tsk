use crate::context::file_system::FileSystemOperations;
use crate::storage::XdgDirectories;
use crate::task::{Task, TaskStatus};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

// Trait for task storage abstraction
#[async_trait::async_trait]
pub trait TaskStorage: Send + Sync {
    async fn add_task(&self, task: Task) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    #[allow(dead_code)] // Will be used in future functionality (e.g., task details view)
    async fn get_task(
        &self,
        id: &str,
    ) -> Result<Option<Task>, Box<dyn std::error::Error + Send + Sync>>;
    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error + Send + Sync>>;
    async fn update_task(&self, task: Task)
    -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn delete_task(&self, id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn delete_tasks_by_status(
        &self,
        statuses: Vec<TaskStatus>,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>>;
}

// JSON file-based implementation
pub struct JsonTaskStorage {
    file_path: PathBuf,
    lock: Arc<Mutex<()>>,
    file_system: Arc<dyn FileSystemOperations>,
    #[allow(dead_code)]
    xdg_directories: Arc<XdgDirectories>,
}

impl JsonTaskStorage {
    pub fn new(
        xdg_directories: Arc<XdgDirectories>,
        file_system: Arc<dyn FileSystemOperations>,
    ) -> Self {
        let file_path = xdg_directories.tasks_file();

        Self {
            file_path,
            lock: Arc::new(Mutex::new(())),
            file_system,
            xdg_directories,
        }
    }

    async fn read_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error + Send + Sync>> {
        if !self
            .file_system
            .exists(&self.file_path)
            .await
            .map_err(|e| e.to_string())?
        {
            return Ok(Vec::new());
        }

        let contents = self
            .file_system
            .read_file(&self.file_path)
            .await
            .map_err(|e| e.to_string())?;
        let tasks: Vec<Task> = serde_json::from_str(&contents)?;
        Ok(tasks)
    }

    async fn write_tasks(
        &self,
        tasks: &[Task],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let contents = serde_json::to_string_pretty(tasks)?;
        self.file_system
            .write_file(&self.file_path, &contents)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl TaskStorage for JsonTaskStorage {
    async fn add_task(&self, task: Task) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _lock = self.lock.lock().await;

        let mut tasks = self.read_tasks().await?;
        tasks.push(task);
        self.write_tasks(&tasks).await?;

        Ok(())
    }

    async fn get_task(
        &self,
        id: &str,
    ) -> Result<Option<Task>, Box<dyn std::error::Error + Send + Sync>> {
        let tasks = self.read_tasks().await?;
        Ok(tasks.into_iter().find(|t| t.id == id))
    }

    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error + Send + Sync>> {
        self.read_tasks().await
    }

    async fn update_task(
        &self,
        task: Task,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

    async fn delete_task(&self, id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
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
pub fn get_task_storage(
    xdg_directories: Arc<XdgDirectories>,
    file_system: Arc<dyn FileSystemOperations>,
) -> Box<dyn TaskStorage> {
    let storage = JsonTaskStorage::new(xdg_directories, file_system);
    Box::new(storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::file_system::DefaultFileSystem;
    use crate::task::Task;
    use tempfile::TempDir;

    fn create_test_xdg_directories(temp_dir: &TempDir) -> Arc<XdgDirectories> {
        let config = crate::storage::XdgConfig::with_paths(
            temp_dir.path().join("data"),
            temp_dir.path().join("runtime"),
            temp_dir.path().join("config"),
        );
        let xdg = XdgDirectories::new(Some(config)).unwrap();
        xdg.ensure_directories().unwrap();
        Arc::new(xdg)
    }

    #[tokio::test]
    async fn test_json_task_storage() {
        let temp_dir = TempDir::new().unwrap();
        let file_system = Arc::new(DefaultFileSystem);
        let xdg_directories = create_test_xdg_directories(&temp_dir);
        let storage = JsonTaskStorage::new(xdg_directories.clone(), file_system);

        // Test adding a task
        let task = Task::new(
            "2025-06-26-0900-feature-test-task".to_string(),
            temp_dir.path().to_path_buf(),
            "test-task".to_string(),
            "feature".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            "tsk/test-task".to_string(),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            temp_dir.path().to_path_buf(),
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
            "2025-06-26-0901-feature-task1".to_string(),
            temp_dir.path().to_path_buf(),
            "task1".to_string(),
            "feature".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            "tsk/task1".to_string(),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            temp_dir.path().to_path_buf(),
        );
        let mut task2 = Task::new(
            "2025-06-26-0902-bug-fix-task2".to_string(),
            temp_dir.path().to_path_buf(),
            "task2".to_string(),
            "bug-fix".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            "tsk/task2".to_string(),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            temp_dir.path().to_path_buf(),
        );
        task2.status = TaskStatus::Complete;
        let mut task3 = Task::new(
            "2025-06-26-0903-refactor-task3".to_string(),
            temp_dir.path().to_path_buf(),
            "task3".to_string(),
            "refactor".to_string(),
            "instructions.md".to_string(),
            "claude-code".to_string(),
            30,
            "tsk/task3".to_string(),
            "abc123".to_string(),
            "default".to_string(),
            "default".to_string(),
            chrono::Local::now(),
            temp_dir.path().to_path_buf(),
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
