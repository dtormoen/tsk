pub mod docker_client;
pub mod file_system;
pub mod git_operations;
pub mod terminal;
pub mod tsk_client;
pub mod tsk_config;

use crate::context::tsk_config::TskConfig;
use crate::git_sync::GitSyncManager;
use crate::notifications::NotificationClient;
use docker_client::DockerClient;
use file_system::FileSystemOperations;
use git_operations::GitOperations;
use terminal::TerminalOperations;

use std::sync::Arc;
use tsk_client::TskClient;

#[cfg(test)]
use crate::test_utils::{NoOpDockerClient, NoOpTskClient};
#[cfg(test)]
use tempfile::TempDir;

// Re-export terminal trait for tests
#[cfg(test)]
pub use terminal::TerminalOperations as TerminalOperationsTrait;

#[derive(Clone)]
pub struct AppContext {
    docker_client: Arc<dyn DockerClient>,
    file_system: Arc<dyn FileSystemOperations>,
    git_operations: Arc<dyn GitOperations>,
    git_sync_manager: Arc<GitSyncManager>,
    notification_client: Arc<dyn NotificationClient>,
    terminal_operations: Arc<dyn TerminalOperations>,
    tsk_client: Arc<dyn TskClient>,
    tsk_config: Arc<TskConfig>,
    #[cfg(test)]
    _temp_dir: Option<Arc<TempDir>>,
}

impl AppContext {
    pub fn builder() -> AppContextBuilder {
        AppContextBuilder::new()
    }

    pub fn docker_client(&self) -> Arc<dyn DockerClient> {
        Arc::clone(&self.docker_client)
    }

    pub fn file_system(&self) -> Arc<dyn FileSystemOperations> {
        Arc::clone(&self.file_system)
    }

    pub fn git_operations(&self) -> Arc<dyn GitOperations> {
        Arc::clone(&self.git_operations)
    }

    pub fn git_sync_manager(&self) -> Arc<GitSyncManager> {
        Arc::clone(&self.git_sync_manager)
    }

    pub fn notification_client(&self) -> Arc<dyn NotificationClient> {
        Arc::clone(&self.notification_client)
    }

    pub fn terminal_operations(&self) -> Arc<dyn TerminalOperations> {
        Arc::clone(&self.terminal_operations)
    }

    pub fn tsk_client(&self) -> Arc<dyn TskClient> {
        Arc::clone(&self.tsk_client)
    }

    pub fn tsk_config(&self) -> Arc<TskConfig> {
        Arc::clone(&self.tsk_config)
    }
}

pub struct AppContextBuilder {
    docker_client: Option<Arc<dyn DockerClient>>,
    file_system: Option<Arc<dyn FileSystemOperations>>,
    git_operations: Option<Arc<dyn GitOperations>>,
    git_sync_manager: Option<Arc<GitSyncManager>>,
    notification_client: Option<Arc<dyn NotificationClient>>,
    terminal_operations: Option<Arc<dyn TerminalOperations>>,
    tsk_client: Option<Arc<dyn TskClient>>,
    tsk_config: Option<Arc<TskConfig>>,
}

impl Default for AppContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)] // Builder methods are used throughout the codebase
impl AppContextBuilder {
    pub fn new() -> Self {
        Self {
            docker_client: None,
            file_system: None,
            git_operations: None,
            git_sync_manager: None,
            notification_client: None,
            terminal_operations: None,
            tsk_client: None,
            tsk_config: None,
        }
    }

    /// Configure the Docker client for this context
    ///
    /// Used extensively in tests and production code throughout the codebase
    pub fn with_docker_client(mut self, docker_client: Arc<dyn DockerClient>) -> Self {
        self.docker_client = Some(docker_client);
        self
    }

    /// Configure the file system operations for this context
    ///
    /// Used extensively in tests and production code throughout the codebase
    pub fn with_file_system(mut self, file_system: Arc<dyn FileSystemOperations>) -> Self {
        self.file_system = Some(file_system);
        self
    }

    /// Configure the git operations for this context
    ///
    /// Used extensively in tests and production code throughout the codebase
    pub fn with_git_operations(mut self, git_operations: Arc<dyn GitOperations>) -> Self {
        self.git_operations = Some(git_operations);
        self
    }

    /// Configure the terminal operations for this context
    ///
    /// Used extensively in tests and production code throughout the codebase
    pub fn with_terminal_operations(
        mut self,
        terminal_operations: Arc<dyn TerminalOperations>,
    ) -> Self {
        self.terminal_operations = Some(terminal_operations);
        self
    }

    /// Configure the TSK client for this context
    ///
    /// Used extensively in tests and production code throughout the codebase
    pub fn with_tsk_client(mut self, tsk_client: Arc<dyn TskClient>) -> Self {
        self.tsk_client = Some(tsk_client);
        self
    }

    /// Configure the TSK configuration for this context
    ///
    /// Used extensively in tests and production code throughout the codebase
    pub fn with_tsk_config(mut self, tsk_config: Arc<TskConfig>) -> Self {
        self.tsk_config = Some(tsk_config);
        self
    }

    pub fn build(self) -> AppContext {
        #[cfg(test)]
        {
            // In test mode, automatically create test-safe temporary directories and mocks
            let temp_dir = Arc::new(TempDir::new().expect("Failed to create temp dir"));
            let temp_path = temp_dir.path();

            let tsk_config = self.tsk_config.unwrap_or_else(|| {
                // Create test-safe TSK configuration in temp directory
                let xdg_config = crate::context::tsk_config::XdgConfig::builder()
                    .with_data_dir(temp_path.join("data").to_path_buf())
                    .with_runtime_dir(temp_path.join("runtime").to_path_buf())
                    .with_config_dir(temp_path.join("config").to_path_buf())
                    .with_claude_config_dir(temp_path.join("claude").to_path_buf())
                    .with_git_user_name("Test User".to_string())
                    .with_git_user_email("test@example.com".to_string())
                    .build();
                let config = TskConfig::new(Some(xdg_config))
                    .expect("Failed to initialize test TSK configuration");
                config
                    .ensure_directories()
                    .expect("Failed to create test TSK configuration");
                Arc::new(config)
            });

            let tsk_client = self.tsk_client.unwrap_or_else(|| {
                // Use NoOpTskClient by default in tests
                Arc::new(NoOpTskClient)
            });

            let docker_client = self.docker_client.unwrap_or_else(|| {
                // Use NoOpDockerClient by default in tests
                Arc::new(NoOpDockerClient)
            });

            let file_system = self
                .file_system
                .unwrap_or_else(|| Arc::new(file_system::DefaultFileSystem));

            AppContext {
                docker_client,
                file_system,
                git_operations: self
                    .git_operations
                    .unwrap_or_else(|| Arc::new(git_operations::DefaultGitOperations)),
                git_sync_manager: self
                    .git_sync_manager
                    .unwrap_or_else(|| Arc::new(GitSyncManager::new())),
                notification_client: self
                    .notification_client
                    .unwrap_or_else(|| crate::notifications::create_notification_client()),
                terminal_operations: self
                    .terminal_operations
                    .unwrap_or_else(|| Arc::new(terminal::DefaultTerminalOperations::new())),
                tsk_client,
                tsk_config,
                _temp_dir: Some(temp_dir),
            }
        }

        #[cfg(not(test))]
        {
            let tsk_config = self.tsk_config.unwrap_or_else(|| {
                let config = TskConfig::new(None).expect("Failed to initialize TSK configuration");
                // Ensure directories exist
                config
                    .ensure_directories()
                    .expect("Failed to create TSK configuration");
                Arc::new(config)
            });

            let tsk_client = self
                .tsk_client
                .unwrap_or_else(|| Arc::new(tsk_client::DefaultTskClient::new(tsk_config.clone())));

            let docker_client = self
                .docker_client
                .unwrap_or_else(|| Arc::new(docker_client::DefaultDockerClient::new()));

            let file_system = self
                .file_system
                .unwrap_or_else(|| Arc::new(file_system::DefaultFileSystem));

            AppContext {
                docker_client,
                file_system,
                git_operations: self
                    .git_operations
                    .unwrap_or_else(|| Arc::new(git_operations::DefaultGitOperations)),
                git_sync_manager: self
                    .git_sync_manager
                    .unwrap_or_else(|| Arc::new(GitSyncManager::new())),
                notification_client: self
                    .notification_client
                    .unwrap_or_else(|| crate::notifications::create_notification_client()),
                terminal_operations: self
                    .terminal_operations
                    .unwrap_or_else(|| Arc::new(terminal::DefaultTerminalOperations::new())),
                tsk_client,
                tsk_config,
            }
        }
    }
}

#[cfg(test)]
impl AppContext {
    pub fn new_with_test_docker(docker_client: Arc<dyn DockerClient>) -> Self {
        AppContextBuilder::new()
            .with_docker_client(docker_client)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::FixedResponseDockerClient;
    use std::sync::Arc;

    #[test]
    fn test_app_context_creation() {
        let docker_client = Arc::new(FixedResponseDockerClient::default());
        let app_context = AppContext::builder()
            .with_docker_client(docker_client)
            .build();

        // Verify we can get the docker client back
        let client = app_context.docker_client();
        assert!(client.as_any().is::<FixedResponseDockerClient>());
    }

    #[tokio::test]
    async fn test_app_context_with_tsk_client() {
        use crate::test_utils::NoOpTskClient;

        let tsk_client = Arc::new(NoOpTskClient);
        let app_context = AppContext::builder()
            .with_tsk_client(tsk_client.clone())
            .build();

        // Verify we can use the TSK client
        let client = app_context.tsk_client();
        assert!(!client.is_server_available().await);
        assert!(client.list_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_app_context_docker_client_usage() {
        let docker_client = Arc::new(FixedResponseDockerClient::default());
        let app_context = AppContext::builder()
            .with_docker_client(docker_client.clone())
            .build();

        // Test that we can use the docker client through the context
        let client = app_context.docker_client();
        let container_id = client
            .create_container(None, bollard::models::ContainerCreateBody::default())
            .await
            .unwrap();

        assert_eq!(container_id, "test-container-id");
    }

    #[test]
    fn test_app_context_test_constructor() {
        let docker_client = Arc::new(FixedResponseDockerClient::default());
        let app_context = AppContext::new_with_test_docker(docker_client);

        // Verify we can get the docker client back
        let client = app_context.docker_client();
        assert!(client.as_any().is::<FixedResponseDockerClient>());
    }

    #[tokio::test]
    async fn test_app_context_with_file_system() {
        use crate::context::file_system::DefaultFileSystem;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let docker_client = Arc::new(FixedResponseDockerClient::default());
        let file_system = Arc::new(DefaultFileSystem);

        let app_context = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(file_system.clone())
            .build();

        // Create a test file
        let test_file = temp_dir.path().join("file.txt");
        let fs = app_context.file_system();
        fs.write_file(&test_file, "test content").await.unwrap();

        // Verify we can use the file system
        let content = fs.read_file(&test_file).await.unwrap();
        assert_eq!(content, "test content");
    }

    #[test]
    fn test_app_context_with_terminal_operations() {
        use std::sync::Mutex;

        // Create a mock terminal operations implementation
        #[derive(Default)]
        struct MockTerminalOperations {
            titles: Mutex<Vec<String>>,
            restore_called: Mutex<bool>,
        }

        impl TerminalOperations for MockTerminalOperations {
            fn set_title(&self, title: &str) {
                self.titles.lock().unwrap().push(title.to_string());
            }

            fn restore_title(&self) {
                *self.restore_called.lock().unwrap() = true;
            }
        }

        let mock_terminal = Arc::new(MockTerminalOperations::default());
        let app_context = AppContext::builder()
            .with_terminal_operations(mock_terminal.clone())
            .build();

        // Use terminal operations through context
        let terminal = app_context.terminal_operations();
        terminal.set_title("Test Title 1");
        terminal.set_title("Test Title 2");
        terminal.restore_title();

        // Verify the mock recorded the calls
        assert_eq!(
            *mock_terminal.titles.lock().unwrap(),
            vec!["Test Title 1", "Test Title 2"]
        );
        assert!(*mock_terminal.restore_called.lock().unwrap());
    }
}
