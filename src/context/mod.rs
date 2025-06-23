pub mod docker_client;
pub mod file_system;
pub mod git_operations;
pub mod repository_context;
pub mod terminal;
pub mod tsk_client;

#[cfg(test)]
mod tests;

use crate::assets::{layered::LayeredAssetManager, AssetManager};
use crate::docker::image_manager::DockerImageManager;
use crate::notifications::NotificationClient;
use crate::repo_utils::find_repository_root;
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
    asset_manager: Arc<dyn AssetManager>,
    docker_client: Arc<dyn DockerClient>,
    docker_image_manager: Arc<DockerImageManager>,
    file_system: Arc<dyn FileSystemOperations>,
    git_operations: Arc<dyn GitOperations>,
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

    pub fn asset_manager(&self) -> Arc<dyn AssetManager> {
        Arc::clone(&self.asset_manager)
    }

    pub fn docker_client(&self) -> Arc<dyn DockerClient> {
        Arc::clone(&self.docker_client)
    }

    pub fn docker_image_manager(&self) -> Arc<DockerImageManager> {
        Arc::clone(&self.docker_image_manager)
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
    asset_manager: Option<Arc<dyn AssetManager>>,
    docker_client: Option<Arc<dyn DockerClient>>,
    docker_image_manager: Option<Arc<DockerImageManager>>,
    file_system: Option<Arc<dyn FileSystemOperations>>,
    git_operations: Option<Arc<dyn GitOperations>>,
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

impl AppContextBuilder {
    pub fn new() -> Self {
        Self {
            asset_manager: None,
            docker_client: None,
            docker_image_manager: None,
            file_system: None,
            git_operations: None,
            notification_client: None,
            repository_context: None,
            terminal_operations: None,
            tsk_client: None,
            xdg_directories: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_asset_manager(mut self, asset_manager: Arc<dyn AssetManager>) -> Self {
        self.asset_manager = Some(asset_manager);
        self
    }

    #[allow(dead_code)]
    pub fn with_docker_client(mut self, docker_client: Arc<dyn DockerClient>) -> Self {
        self.docker_client = Some(docker_client);
        self
    }

    #[allow(dead_code)]
    pub fn with_docker_image_manager(
        mut self,
        docker_image_manager: Arc<DockerImageManager>,
    ) -> Self {
        self.docker_image_manager = Some(docker_image_manager);
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
    pub fn with_repository_context(
        mut self,
        repository_context: Arc<dyn RepositoryContext>,
    ) -> Self {
        self.repository_context = Some(repository_context);
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

        let docker_client = self
            .docker_client
            .unwrap_or_else(|| Arc::new(docker_client::DefaultDockerClient::new()));

        let asset_manager = self.asset_manager.unwrap_or_else(|| {
            // Try to find the repository root for project-level templates
            let project_root = find_repository_root(std::path::Path::new(".")).ok();
            Arc::new(LayeredAssetManager::new_with_standard_layers(
                project_root.as_deref(),
                &xdg_directories,
            ))
        });

        let docker_image_manager = self.docker_image_manager.unwrap_or_else(|| {
            use crate::docker::composer::DockerComposer;
            use crate::docker::template_manager::DockerTemplateManager;

            // Try to find the repository root for project-level templates
            let project_root = find_repository_root(std::path::Path::new(".")).ok();

            let template_manager = DockerTemplateManager::new(
                asset_manager.clone(),
                xdg_directories.clone(),
                project_root.clone(),
            );

            let composer = DockerComposer::new(DockerTemplateManager::new(
                asset_manager.clone(),
                xdg_directories.clone(),
                project_root,
            ));

            Arc::new(DockerImageManager::new(
                docker_client.clone(),
                template_manager,
                composer,
            ))
        });

        let file_system = self
            .file_system
            .unwrap_or_else(|| Arc::new(file_system::DefaultFileSystem));

        let repository_context = self.repository_context.unwrap_or_else(|| {
            Arc::new(repository_context::DefaultRepositoryContext::new(
                file_system.clone(),
            ))
        });

        AppContext {
            asset_manager,
            docker_client,
            docker_image_manager,
            file_system,
            git_operations: self
                .git_operations
                .unwrap_or_else(|| Arc::new(git_operations::DefaultGitOperations)),
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
