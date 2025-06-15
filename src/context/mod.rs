pub mod docker_client;
pub mod file_system;
pub mod git_operations;
pub mod terminal;
pub mod tsk_client;

#[cfg(test)]
mod tests;

use crate::notifications::NotificationClient;
use crate::storage::XdgDirectories;
use docker_client::DockerClient;
use file_system::FileSystemOperations;
use git_operations::GitOperations;
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
    notification_client: Arc<dyn NotificationClient>,
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

    pub fn notification_client(&self) -> Arc<dyn NotificationClient> {
        Arc::clone(&self.notification_client)
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
    notification_client: Option<Arc<dyn NotificationClient>>,
    terminal_operations: Option<Arc<dyn TerminalOperations>>,
    tsk_client: Option<Arc<dyn TskClient>>,
    xdg_directories: Option<Arc<XdgDirectories>>,
}

impl AppContextBuilder {
    pub fn new() -> Self {
        Self {
            docker_client: None,
            file_system: None,
            git_operations: None,
            notification_client: None,
            terminal_operations: None,
            tsk_client: None,
            xdg_directories: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_docker_client(mut self, docker_client: Arc<dyn DockerClient>) -> Self {
        self.docker_client = Some(docker_client);
        self
    }

    #[allow(dead_code)]
    pub fn with_file_system(mut self, file_system: Arc<dyn FileSystemOperations>) -> Self {
        self.file_system = Some(file_system);
        self
    }

    #[allow(dead_code)]
    pub fn with_git_operations(mut self, git_operations: Arc<dyn GitOperations>) -> Self {
        self.git_operations = Some(git_operations);
        self
    }

    #[allow(dead_code)]
    pub fn with_notification_client(
        mut self,
        notification_client: Arc<dyn NotificationClient>,
    ) -> Self {
        self.notification_client = Some(notification_client);
        self
    }

    #[allow(dead_code)]
    pub fn with_terminal_operations(
        mut self,
        terminal_operations: Arc<dyn TerminalOperations>,
    ) -> Self {
        self.terminal_operations = Some(terminal_operations);
        self
    }

    #[allow(dead_code)]
    pub fn with_tsk_client(mut self, tsk_client: Arc<dyn TskClient>) -> Self {
        self.tsk_client = Some(tsk_client);
        self
    }

    #[allow(dead_code)]
    pub fn with_xdg_directories(mut self, xdg_directories: Arc<XdgDirectories>) -> Self {
        self.xdg_directories = Some(xdg_directories);
        self
    }

    pub fn build(self) -> AppContext {
        let xdg_directories = self.xdg_directories.unwrap_or_else(|| {
            let xdg = XdgDirectories::new().expect("Failed to initialize XDG directories");
            // Ensure directories exist
            xdg.ensure_directories()
                .expect("Failed to create XDG directories");
            Arc::new(xdg)
        });

        let tsk_client = self.tsk_client.unwrap_or_else(|| {
            Arc::new(tsk_client::DefaultTskClient::new(xdg_directories.clone()))
        });

        AppContext {
            docker_client: self
                .docker_client
                .unwrap_or_else(|| Arc::new(docker_client::DefaultDockerClient::new())),
            file_system: self
                .file_system
                .unwrap_or_else(|| Arc::new(file_system::DefaultFileSystem)),
            git_operations: self
                .git_operations
                .unwrap_or_else(|| Arc::new(git_operations::DefaultGitOperations)),
            notification_client: self
                .notification_client
                .unwrap_or_else(|| crate::notifications::create_notification_client()),
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
