pub mod docker_client;
pub mod file_system;
pub mod git_operations;
pub mod repository_context;
pub mod terminal;
pub mod tsk_client;

use crate::git_sync::GitSyncManager;
use crate::notifications::NotificationClient;
use crate::storage::XdgDirectories;
use docker_client::DockerClient;
use file_system::FileSystemOperations;
use git_operations::GitOperations;
use repository_context::RepositoryContext;
use terminal::TerminalOperations;

use std::sync::Arc;
use tsk_client::TskClient;

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
    repository_context: Arc<dyn RepositoryContext>,
    terminal_operations: Arc<dyn TerminalOperations>,
    tsk_client: Arc<dyn TskClient>,
    xdg_directories: Arc<XdgDirectories>,
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

    pub fn repository_context(&self) -> Arc<dyn RepositoryContext> {
        Arc::clone(&self.repository_context)
    }

    pub fn terminal_operations(&self) -> Arc<dyn TerminalOperations> {
        Arc::clone(&self.terminal_operations)
    }

    pub fn tsk_client(&self) -> Arc<dyn TskClient> {
        Arc::clone(&self.tsk_client)
    }

    pub fn xdg_directories(&self) -> Arc<XdgDirectories> {
        Arc::clone(&self.xdg_directories)
    }
}

pub struct AppContextBuilder {
    docker_client: Option<Arc<dyn DockerClient>>,
    file_system: Option<Arc<dyn FileSystemOperations>>,
    git_operations: Option<Arc<dyn GitOperations>>,
    git_sync_manager: Option<Arc<GitSyncManager>>,
    notification_client: Option<Arc<dyn NotificationClient>>,
    repository_context: Option<Arc<dyn RepositoryContext>>,
    terminal_operations: Option<Arc<dyn TerminalOperations>>,
    tsk_client: Option<Arc<dyn TskClient>>,
    xdg_directories: Option<Arc<XdgDirectories>>,
}

impl Default for AppContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl AppContextBuilder {
    pub fn new() -> Self {
        Self {
            docker_client: None,
            file_system: None,
            git_operations: None,
            git_sync_manager: None,
            notification_client: None,
            repository_context: None,
            terminal_operations: None,
            tsk_client: None,
            xdg_directories: None,
        }
    }

    pub fn with_docker_client(mut self, docker_client: Arc<dyn DockerClient>) -> Self {
        self.docker_client = Some(docker_client);
        self
    }

    pub fn with_file_system(mut self, file_system: Arc<dyn FileSystemOperations>) -> Self {
        self.file_system = Some(file_system);
        self
    }

    pub fn with_git_operations(mut self, git_operations: Arc<dyn GitOperations>) -> Self {
        self.git_operations = Some(git_operations);
        self
    }

    pub fn with_terminal_operations(
        mut self,
        terminal_operations: Arc<dyn TerminalOperations>,
    ) -> Self {
        self.terminal_operations = Some(terminal_operations);
        self
    }

    pub fn with_tsk_client(mut self, tsk_client: Arc<dyn TskClient>) -> Self {
        self.tsk_client = Some(tsk_client);
        self
    }

    pub fn with_xdg_directories(mut self, xdg_directories: Arc<XdgDirectories>) -> Self {
        self.xdg_directories = Some(xdg_directories);
        self
    }

    pub fn build(self) -> AppContext {
        let xdg_directories = self.xdg_directories.unwrap_or_else(|| {
            let xdg = XdgDirectories::new(None).expect("Failed to initialize XDG directories");
            // Ensure directories exist
            xdg.ensure_directories()
                .expect("Failed to create XDG directories");
            Arc::new(xdg)
        });

        let tsk_client = self.tsk_client.unwrap_or_else(|| {
            Arc::new(tsk_client::DefaultTskClient::new(xdg_directories.clone()))
        });

        let docker_client = self
            .docker_client
            .unwrap_or_else(|| Arc::new(docker_client::DefaultDockerClient::new()));

        let file_system = self
            .file_system
            .unwrap_or_else(|| Arc::new(file_system::DefaultFileSystem));

        let repository_context = self.repository_context.unwrap_or_else(|| {
            Arc::new(repository_context::DefaultRepositoryContext::new(
                file_system.clone(),
            ))
        });

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
            repository_context,
            terminal_operations: self
                .terminal_operations
                .unwrap_or_else(|| Arc::new(terminal::DefaultTerminalOperations::new())),
            tsk_client,
            xdg_directories,
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
            .create_container(None, bollard::container::Config::default())
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
        use crate::context::file_system::tests::MockFileSystem;

        let docker_client = Arc::new(FixedResponseDockerClient::default());
        let file_system =
            Arc::new(MockFileSystem::new().with_file("/test/file.txt", "test content"));

        let app_context = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(file_system.clone())
            .build();

        // Verify we can use the file system
        let fs = app_context.file_system();
        let content = fs
            .read_file(std::path::Path::new("/test/file.txt"))
            .await
            .unwrap();
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
