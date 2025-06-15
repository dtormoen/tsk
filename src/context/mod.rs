pub mod docker_client;
pub mod file_system;
pub mod git_operations;

#[cfg(test)]
mod tests;

use crate::notifications::NotificationClient;
use docker_client::DockerClient;
use file_system::FileSystemOperations;
use git_operations::GitOperations;
use std::sync::Arc;

pub struct AppContext {
    docker_client: Arc<dyn DockerClient>,
    file_system: Arc<dyn FileSystemOperations>,
    git_operations: Arc<dyn GitOperations>,
    notification_client: Arc<dyn NotificationClient>,
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
}

pub struct AppContextBuilder {
    docker_client: Option<Arc<dyn DockerClient>>,
    file_system: Option<Arc<dyn FileSystemOperations>>,
    git_operations: Option<Arc<dyn GitOperations>>,
    notification_client: Option<Arc<dyn NotificationClient>>,
}

impl AppContextBuilder {
    pub fn new() -> Self {
        Self {
            docker_client: None,
            file_system: None,
            git_operations: None,
            notification_client: None,
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

    pub fn build(self) -> AppContext {
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
