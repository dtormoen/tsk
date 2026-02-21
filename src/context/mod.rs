pub mod docker_client;
pub mod notifications;
pub mod terminal;
pub mod tsk_config;
pub mod tsk_env;

// Re-export TskConfig types from tsk_config module
// Types used in production code
pub use tsk_config::{ContainerEngine, TskConfig, VolumeMount};
// Types only used in tests
#[cfg(test)]
pub use tsk_config::{BindMount, DockerOptions, EnvVar, NamedVolume, ProjectConfig};

// Re-export TskEnv types
pub use tsk_env::TskEnv;

use crate::git_sync::GitSyncManager;
use crate::task_storage::TaskStorage;
use notifications::NotificationClient;

use std::sync::Arc;

#[cfg(test)]
use tempfile::TempDir;

#[derive(Clone)]
pub struct AppContext {
    git_sync_manager: Arc<GitSyncManager>,
    notification_client: Arc<NotificationClient>,
    task_storage: Arc<dyn TaskStorage>,
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

    pub fn git_sync_manager(&self) -> Arc<GitSyncManager> {
        Arc::clone(&self.git_sync_manager)
    }

    pub fn notification_client(&self) -> Arc<NotificationClient> {
        Arc::clone(&self.notification_client)
    }

    pub fn task_storage(&self) -> Arc<dyn TaskStorage> {
        Arc::clone(&self.task_storage)
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
    container_engine: Option<ContainerEngine>,
    git_sync_manager: Option<Arc<GitSyncManager>>,
    notification_client: Option<Arc<NotificationClient>>,
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
            container_engine: None,
            git_sync_manager: None,
            notification_client: None,
            tsk_config: None,
            tsk_env: None,
        }
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

    /// Set the container engine (CLI override)
    ///
    /// When set, overrides the container_engine from tsk.toml config.
    pub fn with_container_engine(mut self, engine: Option<ContainerEngine>) -> Self {
        self.container_engine = engine;
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

            let task_storage = crate::task_storage::get_task_storage(tsk_env.clone());

            AppContext {
                git_sync_manager: self
                    .git_sync_manager
                    .unwrap_or_else(|| Arc::new(GitSyncManager::new())),
                notification_client: self
                    .notification_client
                    .unwrap_or_else(notifications::create_notification_client),
                task_storage,
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
            let tsk_config = self.tsk_config.unwrap_or_else(|| {
                let mut config = tsk_config::load_config(tsk_env.config_dir());
                // CLI override for container engine
                if let Some(engine) = self.container_engine {
                    config.docker.container_engine = engine;
                }
                Arc::new(config)
            });

            let task_storage = crate::task_storage::get_task_storage(tsk_env.clone());

            AppContext {
                git_sync_manager: self
                    .git_sync_manager
                    .unwrap_or_else(|| Arc::new(GitSyncManager::new())),
                notification_client: self
                    .notification_client
                    .unwrap_or_else(notifications::create_notification_client),
                task_storage,
                terminal_operations: Arc::new(terminal::TerminalOperations::new()),
                tsk_config,
                tsk_env,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_context_creation() {
        let app_context = AppContext::builder().build();

        // Verify we can access all expected fields
        let _env = app_context.tsk_env();
        let _config = app_context.tsk_config();
        let _git_sync = app_context.git_sync_manager();
        let _notification = app_context.notification_client();
        let _terminal = app_context.terminal_operations();
        let _storage = app_context.task_storage();
    }
}
