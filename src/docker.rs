use async_trait::async_trait;
use bollard::container::{Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions};
use bollard::Docker;
use std::path::{Path, PathBuf};

/// Factory function to get a DockerManager instance
/// Returns a dummy implementation in test mode that panics on use
#[cfg(not(test))]
pub fn get_docker_manager() -> Result<DockerManager<RealDockerClient>, String> {
    DockerManager::new()
}

#[cfg(test)]
pub fn get_docker_manager() -> Result<DockerManager<PanicDockerClient>, String> {
    Ok(DockerManager::with_client(PanicDockerClient))
}

#[cfg(test)]
pub struct PanicDockerClient;

#[cfg(test)]
#[async_trait]
impl DockerClient for PanicDockerClient {
    async fn create_container(
        &self,
        _options: Option<CreateContainerOptions<String>>,
        _config: Config<String>,
    ) -> Result<String, String> {
        panic!("Docker operations are not allowed in tests! Please mock DockerManager properly using DockerManager::with_client()")
    }

    async fn start_container(&self, _id: &str) -> Result<(), String> {
        panic!("Docker operations are not allowed in tests! Please mock DockerManager properly using DockerManager::with_client()")
    }

    async fn wait_container(&self, _id: &str) -> Result<i64, String> {
        panic!("Docker operations are not allowed in tests! Please mock DockerManager properly using DockerManager::with_client()")
    }

    async fn logs(
        &self,
        _id: &str,
        _options: Option<LogsOptions<String>>,
    ) -> Result<String, String> {
        panic!("Docker operations are not allowed in tests! Please mock DockerManager properly using DockerManager::with_client()")
    }

    async fn remove_container(
        &self,
        _id: &str,
        _options: Option<RemoveContainerOptions>,
    ) -> Result<(), String> {
        panic!("Docker operations are not allowed in tests! Please mock DockerManager properly using DockerManager::with_client()")
    }
}

// Container resource limits
const CONTAINER_MEMORY_LIMIT: i64 = 2 * 1024 * 1024 * 1024; // 2GB
const CONTAINER_CPU_QUOTA: i64 = 100000; // 1 CPU
const CONTAINER_NETWORK_MODE: &str = "none";
const CONTAINER_WORKING_DIR: &str = "/workspace";
const CONTAINER_USER: &str = "agent";

#[async_trait]
pub trait DockerClient: Send + Sync {
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions<String>>,
        config: Config<String>,
    ) -> Result<String, String>;

    async fn start_container(&self, id: &str) -> Result<(), String>;

    async fn wait_container(&self, id: &str) -> Result<i64, String>;

    async fn logs(&self, id: &str, options: Option<LogsOptions<String>>) -> Result<String, String>;

    async fn remove_container(
        &self,
        id: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), String>;
}

pub struct RealDockerClient {
    docker: Docker,
}

impl RealDockerClient {
    pub fn new() -> Result<Self, String> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| format!("Failed to connect to Docker: {}", e))?;
        Ok(Self { docker })
    }
}

#[async_trait]
impl DockerClient for RealDockerClient {
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions<String>>,
        config: Config<String>,
    ) -> Result<String, String> {
        let response = self
            .docker
            .create_container(options, config)
            .await
            .map_err(|e| format!("Failed to create container: {}", e))?;
        Ok(response.id)
    }

    async fn start_container(&self, id: &str) -> Result<(), String> {
        self.docker
            .start_container::<String>(id, None)
            .await
            .map_err(|e| format!("Failed to start container: {}", e))
    }

    async fn wait_container(&self, id: &str) -> Result<i64, String> {
        use futures_util::stream::StreamExt;

        let mut stream = self
            .docker
            .wait_container(id, None::<bollard::container::WaitContainerOptions<String>>);
        if let Some(result) = stream.next().await {
            match result {
                Ok(wait_response) => Ok(wait_response.status_code),
                Err(e) => Err(format!("Failed to wait for container: {}", e)),
            }
        } else {
            Err("Container wait stream ended unexpectedly".to_string())
        }
    }

    async fn logs(&self, id: &str, options: Option<LogsOptions<String>>) -> Result<String, String> {
        use futures_util::stream::StreamExt;

        let mut stream = self.docker.logs(id, options);
        let mut output = String::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(log) => output.push_str(&log.to_string()),
                Err(e) => return Err(format!("Failed to get logs: {}", e)),
            }
        }

        Ok(output)
    }

    async fn remove_container(
        &self,
        id: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), String> {
        self.docker
            .remove_container(id, options)
            .await
            .map_err(|e| format!("Failed to remove container: {}", e))
    }
}

pub struct DockerManager<C: DockerClient> {
    client: C,
}

impl DockerManager<RealDockerClient> {
    pub fn new() -> Result<Self, String> {
        let client = RealDockerClient::new()?;
        Ok(Self { client })
    }
}

impl<C: DockerClient> DockerManager<C> {
    pub fn with_client(client: C) -> Self {
        Self { client }
    }

    fn prepare_worktree_path(worktree_path: &Path) -> Result<PathBuf, String> {
        // Convert to absolute path to ensure Docker can find the volume
        let absolute_path = if worktree_path.is_relative() {
            std::env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?
                .join(worktree_path)
        } else {
            worktree_path.to_path_buf()
        };
        Ok(absolute_path)
    }

    fn create_base_container_config(
        image: &str,
        worktree_path_str: &str,
        command: Option<Vec<String>>,
        interactive: bool,
    ) -> Config<String> {
        Config {
            image: Some(image.to_string()),
            cmd: command,
            host_config: Some(bollard::service::HostConfig {
                binds: Some(vec![format!(
                    "{}:{}",
                    worktree_path_str, CONTAINER_WORKING_DIR
                )]),
                network_mode: Some(CONTAINER_NETWORK_MODE.to_string()),
                memory: Some(CONTAINER_MEMORY_LIMIT),
                cpu_quota: Some(CONTAINER_CPU_QUOTA),
                ..Default::default()
            }),
            working_dir: Some(CONTAINER_WORKING_DIR.to_string()),
            user: Some(CONTAINER_USER.to_string()),
            env: Some(vec![
                format!("HOME=/home/{}", CONTAINER_USER),
                format!("USER={}", CONTAINER_USER),
            ]),
            attach_stdin: Some(interactive),
            attach_stdout: Some(interactive),
            attach_stderr: Some(interactive),
            tty: Some(interactive),
            ..Default::default()
        }
    }

    pub async fn create_debug_container(
        &self,
        image: &str,
        worktree_path: &Path,
    ) -> Result<String, String> {
        let absolute_worktree_path = Self::prepare_worktree_path(worktree_path)?;
        let worktree_path_str = absolute_worktree_path
            .to_str()
            .ok_or_else(|| "Invalid worktree path".to_string())?;

        let sleep_command = Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            "sleep infinity".to_string(),
        ]);

        let config = Self::create_base_container_config(
            image,
            worktree_path_str,
            sleep_command,
            true, // interactive
        );

        let container_name = format!("tsk-debug-{}", chrono::Utc::now().timestamp());
        let options = CreateContainerOptions {
            name: container_name.clone(),
            platform: None,
        };

        let container_id = self.client.create_container(Some(options), config).await?;
        self.client.start_container(&container_id).await?;

        Ok(container_name)
    }

    pub async fn stop_and_remove_container(&self, container_name: &str) -> Result<(), String> {
        self.client
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
    }

    pub async fn run_task_container(
        &self,
        image: &str,
        worktree_path: &Path,
        command: Vec<String>,
    ) -> Result<String, String> {
        let absolute_worktree_path = Self::prepare_worktree_path(worktree_path)?;
        let worktree_path_str = absolute_worktree_path
            .to_str()
            .ok_or_else(|| "Invalid worktree path".to_string())?;

        let cmd = if command.is_empty() {
            None
        } else {
            Some(command.clone())
        };

        let config = Self::create_base_container_config(
            image,
            worktree_path_str,
            cmd,
            false, // not interactive
        );

        let options = CreateContainerOptions {
            name: format!("tsk-{}", chrono::Utc::now().timestamp()),
            platform: None,
        };

        let container_id = self.client.create_container(Some(options), config).await?;
        self.client.start_container(&container_id).await?;

        let exit_code = self.client.wait_container(&container_id).await?;

        let logs = self
            .client
            .logs(
                &container_id,
                Some(LogsOptions {
                    stdout: true,
                    stderr: true,
                    ..Default::default()
                }),
            )
            .await?;

        self.client
            .remove_container(
                &container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await?;

        if exit_code != 0 {
            return Err(format!(
                "Container exited with non-zero status: {}. Logs:\n{}",
                exit_code, logs
            ));
        }

        Ok(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockDockerClient {
        create_container_calls:
            Arc<Mutex<Vec<(Option<CreateContainerOptions<String>>, Config<String>)>>>,
        start_container_calls: Arc<Mutex<Vec<String>>>,
        wait_container_calls: Arc<Mutex<Vec<String>>>,
        logs_calls: Arc<Mutex<Vec<(String, Option<LogsOptions<String>>)>>>,
        remove_container_calls: Arc<Mutex<Vec<(String, Option<RemoveContainerOptions>)>>>,

        create_container_result: Arc<Mutex<Result<String, String>>>,
        start_container_result: Arc<Mutex<Result<(), String>>>,
        wait_container_result: Arc<Mutex<Result<i64, String>>>,
        logs_result: Arc<Mutex<Result<String, String>>>,
        remove_container_result: Arc<Mutex<Result<(), String>>>,
    }

    impl MockDockerClient {
        fn new() -> Self {
            Self {
                create_container_calls: Arc::new(Mutex::new(Vec::new())),
                start_container_calls: Arc::new(Mutex::new(Vec::new())),
                wait_container_calls: Arc::new(Mutex::new(Vec::new())),
                logs_calls: Arc::new(Mutex::new(Vec::new())),
                remove_container_calls: Arc::new(Mutex::new(Vec::new())),

                create_container_result: Arc::new(Mutex::new(Ok("test-container-id".to_string()))),
                start_container_result: Arc::new(Mutex::new(Ok(()))),
                wait_container_result: Arc::new(Mutex::new(Ok(0))),
                logs_result: Arc::new(Mutex::new(Ok("Container logs".to_string()))),
                remove_container_result: Arc::new(Mutex::new(Ok(()))),
            }
        }
    }

    #[async_trait]
    impl DockerClient for MockDockerClient {
        async fn create_container(
            &self,
            options: Option<CreateContainerOptions<String>>,
            config: Config<String>,
        ) -> Result<String, String> {
            self.create_container_calls
                .lock()
                .unwrap()
                .push((options, config));
            self.create_container_result.lock().unwrap().clone()
        }

        async fn start_container(&self, id: &str) -> Result<(), String> {
            self.start_container_calls
                .lock()
                .unwrap()
                .push(id.to_string());
            self.start_container_result.lock().unwrap().clone()
        }

        async fn wait_container(&self, id: &str) -> Result<i64, String> {
            self.wait_container_calls
                .lock()
                .unwrap()
                .push(id.to_string());
            self.wait_container_result.lock().unwrap().clone()
        }

        async fn logs(
            &self,
            id: &str,
            options: Option<LogsOptions<String>>,
        ) -> Result<String, String> {
            self.logs_calls
                .lock()
                .unwrap()
                .push((id.to_string(), options));
            self.logs_result.lock().unwrap().clone()
        }

        async fn remove_container(
            &self,
            id: &str,
            options: Option<RemoveContainerOptions>,
        ) -> Result<(), String> {
            self.remove_container_calls
                .lock()
                .unwrap()
                .push((id.to_string(), options));
            self.remove_container_result.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn test_run_task_container_success() {
        let mock_client = MockDockerClient::new();
        let manager = DockerManager::with_client(mock_client);

        let worktree_path = Path::new("/tmp/test-worktree");
        let command = vec!["echo".to_string(), "hello".to_string()];

        let result = manager
            .run_task_container("tsk/base", worktree_path, command.clone())
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Container logs");

        let create_calls = manager.client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 1);
        assert_eq!(create_calls[0].1.cmd, Some(command));
        drop(create_calls); // Release the lock

        let start_calls = manager.client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 1);
        assert_eq!(start_calls[0], "test-container-id");

        let wait_calls = manager.client.wait_container_calls.lock().unwrap();
        assert_eq!(wait_calls.len(), 1);
        assert_eq!(wait_calls[0], "test-container-id");

        let remove_calls = manager.client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, "test-container-id");
    }

    #[tokio::test]
    async fn test_run_task_container_no_command() {
        let mock_client = MockDockerClient::new();
        let manager = DockerManager::with_client(mock_client);

        let worktree_path = Path::new("/tmp/test-worktree");
        let command = vec![];

        let result = manager
            .run_task_container("tsk/base", worktree_path, command)
            .await;

        assert!(result.is_ok());

        // Verify no command was set when empty command array is passed
        let create_calls = manager.client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls[0].1.cmd, None);
    }

    #[tokio::test]
    async fn test_run_task_container_non_zero_exit() {
        let mock_client = MockDockerClient::new();
        *mock_client.wait_container_result.lock().unwrap() = Ok(1);
        let manager = DockerManager::with_client(mock_client);

        let worktree_path = Path::new("/tmp/test-worktree");
        let command = vec!["false".to_string()];

        let result = manager
            .run_task_container("tsk/base", worktree_path, command)
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Container exited with non-zero status: 1"));

        let remove_calls = manager.client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
    }

    #[tokio::test]
    async fn test_run_task_container_create_fails() {
        let mock_client = MockDockerClient::new();
        *mock_client.create_container_result.lock().unwrap() =
            Err("Docker daemon not running".to_string());
        let manager = DockerManager::with_client(mock_client);

        let worktree_path = Path::new("/tmp/test-worktree");
        let command = vec!["echo".to_string(), "hello".to_string()];

        let result = manager
            .run_task_container("tsk/base", worktree_path, command)
            .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Docker daemon not running");

        let start_calls = manager.client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_container_configuration() {
        let mock_client = MockDockerClient::new();
        let manager = DockerManager::with_client(mock_client);

        let worktree_path = Path::new("/tmp/test-worktree");
        let command = vec!["test".to_string()];

        let _ = manager
            .run_task_container("tsk/base", worktree_path, command.clone())
            .await;

        let create_calls = manager.client.create_container_calls.lock().unwrap();
        let (options, config) = &create_calls[0];

        assert!(options.as_ref().unwrap().name.starts_with("tsk-"));
        assert_eq!(config.image, Some("tsk/base".to_string()));
        assert_eq!(config.working_dir, Some(CONTAINER_WORKING_DIR.to_string()));
        assert_eq!(config.user, Some(CONTAINER_USER.to_string()));
        assert_eq!(config.cmd, Some(command));

        let host_config = config.host_config.as_ref().unwrap();
        assert_eq!(
            host_config.network_mode,
            Some(CONTAINER_NETWORK_MODE.to_string())
        );
        assert_eq!(host_config.memory, Some(CONTAINER_MEMORY_LIMIT));
        assert_eq!(host_config.cpu_quota, Some(CONTAINER_CPU_QUOTA));
        assert!(host_config.binds.as_ref().unwrap()[0]
            .contains(&format!("/tmp/test-worktree:{}", CONTAINER_WORKING_DIR)));
    }

    #[tokio::test]
    async fn test_relative_path_conversion() {
        let mock_client = MockDockerClient::new();
        let manager = DockerManager::with_client(mock_client);

        let relative_path = Path::new("test-worktree");
        let command = vec!["test".to_string()];

        let result = manager
            .run_task_container("tsk/base", relative_path, command)
            .await;

        assert!(result.is_ok());

        let create_calls = manager.client.create_container_calls.lock().unwrap();
        let (_, config) = &create_calls[0];

        let host_config = config.host_config.as_ref().unwrap();
        let bind = &host_config.binds.as_ref().unwrap()[0];

        // Should contain an absolute path (starts with /)
        assert!(bind.starts_with('/'));
        assert!(bind.ends_with(&format!("test-worktree:{}", CONTAINER_WORKING_DIR)));
    }
}
