pub mod docker_client;
pub mod terminal;
pub mod tsk_config;
pub mod tsk_env;

// Re-export TskConfig types from tsk_config module
// Types used in production code
pub use tsk_config::{TskConfig, VolumeMount};
// Types only used in tests
#[cfg(test)]
pub use tsk_config::{BindMount, DockerOptions, EnvVar, NamedVolume, ProjectConfig};

// Re-export TskEnv types
pub use tsk_env::TskEnv;

use crate::git_sync::GitSyncManager;
use crate::notifications::NotificationClient;
use docker_client::DockerClient;

use std::sync::Arc;

#[cfg(test)]
use crate::test_utils::NoOpDockerClient;
#[cfg(test)]
use tempfile::TempDir;

#[derive(Clone)]
pub struct AppContext {
    docker_client: Arc<dyn DockerClient>,
    git_sync_manager: Arc<GitSyncManager>,
    notification_client: Arc<dyn NotificationClient>,
    terminal_operations: Arc<terminal::TerminalOperations>,
    tsk_config: Arc<TskConfig>,
    tsk_env: Arc<TskEnv>,
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

    pub fn git_sync_manager(&self) -> Arc<GitSyncManager> {
        Arc::clone(&self.git_sync_manager)
    }

    pub fn notification_client(&self) -> Arc<dyn NotificationClient> {
        Arc::clone(&self.notification_client)
    }

    pub fn terminal_operations(&self) -> Arc<terminal::TerminalOperations> {
        Arc::clone(&self.terminal_operations)
    }

    /// Returns the user configuration loaded from tsk.toml
    pub fn tsk_config(&self) -> Arc<TskConfig> {
        Arc::clone(&self.tsk_config)
    }

    pub fn tsk_env(&self) -> Arc<TskEnv> {
        Arc::clone(&self.tsk_env)
    }
}

pub struct AppContextBuilder {
    docker_client: Option<Arc<dyn DockerClient>>,
    git_sync_manager: Option<Arc<GitSyncManager>>,
    notification_client: Option<Arc<dyn NotificationClient>>,
    tsk_config: Option<Arc<TskConfig>>,
    tsk_env: Option<Arc<TskEnv>>,
}

impl Default for AppContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AppContextBuilder {
    pub fn new() -> Self {
        Self {
            docker_client: None,
            git_sync_manager: None,
            notification_client: None,
            tsk_config: None,
            tsk_env: None,
        }
    }

    /// Configure the Docker client for this context
    ///
    /// Used extensively in tests throughout the codebase
    #[allow(dead_code)] // Used in tests
    pub fn with_docker_client(mut self, docker_client: Arc<dyn DockerClient>) -> Self {
        self.docker_client = Some(docker_client);
        self
    }

    /// Configure the TSK environment for this context
    ///
    /// Used in tests to provide custom TSK environment
    #[allow(dead_code)]
    pub fn with_tsk_env(mut self, tsk_env: Arc<TskEnv>) -> Self {
        self.tsk_env = Some(tsk_env);
        self
    }

    /// Configure the TSK configuration for this context
    ///
    /// Used in tests to provide custom TSK configuration.
    #[allow(dead_code)]
    pub fn with_tsk_config(mut self, config: TskConfig) -> Self {
        self.tsk_config = Some(Arc::new(config));
        self
    }

    pub fn build(self) -> AppContext {
        #[cfg(test)]
        {
            // In test mode, automatically create test-safe temporary directories and mocks
            let temp_dir = Arc::new(TempDir::new().expect("Failed to create temp dir"));
            let temp_path = temp_dir.path();

            let tsk_env = self.tsk_env.unwrap_or_else(|| {
                // Create test-safe TSK environment in temp directory
                let env = TskEnv::builder()
                    .with_data_dir(temp_path.join("data").to_path_buf())
                    .with_runtime_dir(temp_path.join("runtime").to_path_buf())
                    .with_config_dir(temp_path.join("config").to_path_buf())
                    .with_claude_config_dir(temp_path.join("claude").to_path_buf())
                    .build()
                    .expect("Failed to initialize test TSK environment");
                env.ensure_directories()
                    .expect("Failed to create test TSK environment");
                Arc::new(env)
            });

            // Use provided tsk_config or create default
            let tsk_config = self
                .tsk_config
                .unwrap_or_else(|| Arc::new(TskConfig::default()));

            let docker_client = self.docker_client.unwrap_or_else(|| {
                // Use NoOpDockerClient by default in tests
                Arc::new(NoOpDockerClient)
            });

            AppContext {
                docker_client,
                git_sync_manager: self
                    .git_sync_manager
                    .unwrap_or_else(|| Arc::new(GitSyncManager::new())),
                notification_client: self
                    .notification_client
                    .unwrap_or_else(crate::notifications::create_notification_client),
                terminal_operations: Arc::new(terminal::TerminalOperations::new()),
                tsk_config,
                tsk_env,
                _temp_dir: Some(temp_dir),
            }
        }

        #[cfg(not(test))]
        {
            let tsk_env = self.tsk_env.unwrap_or_else(|| {
                let env = TskEnv::new().expect("Failed to initialize TSK environment");
                // Ensure directories exist
                env.ensure_directories()
                    .expect("Failed to create TSK environment");
                Arc::new(env)
            });

            // Load tsk_config from TOML file or use provided/default
            let tsk_config = self
                .tsk_config
                .unwrap_or_else(|| Arc::new(tsk_config::load_config(tsk_env.config_dir())));

            let docker_client = self
                .docker_client
                .unwrap_or_else(|| Arc::new(docker_client::DefaultDockerClient::new()));

            AppContext {
                docker_client,
                git_sync_manager: self
                    .git_sync_manager
                    .unwrap_or_else(|| Arc::new(GitSyncManager::new())),
                notification_client: self
                    .notification_client
                    .unwrap_or_else(crate::notifications::create_notification_client),
                terminal_operations: Arc::new(terminal::TerminalOperations::new()),
                tsk_config,
                tsk_env,
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
}
