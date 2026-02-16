pub mod build_lock_manager;
pub mod composer;
pub mod image_manager;
pub mod layers;
pub mod proxy_manager;
pub mod template_engine;
pub mod template_manager;

use crate::agent::{Agent, LogProcessor};
use crate::context::AppContext;
use crate::context::VolumeMount;
use crate::docker::proxy_manager::ProxyManager;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{LogsOptions, RemoveContainerOptions};
use futures_util::stream::StreamExt;
use std::path::PathBuf;

const CONTAINER_WORKSPACE_BASE: &str = "/workspace";
const CONTAINER_USER: &str = "agent";

fn container_working_dir(project: &str) -> String {
    format!("{CONTAINER_WORKSPACE_BASE}/{project}")
}

/// Manages Docker container execution for TSK tasks.
///
/// This struct handles the lifecycle of task containers including:
/// - Container configuration and creation
/// - Proxy management for network isolation
/// - Log streaming and processing
/// - Container cleanup
pub struct DockerManager {
    ctx: AppContext,
    proxy_manager: ProxyManager,
}

impl DockerManager {
    /// Creates a new DockerManager with the given application context.
    ///
    /// # Arguments
    /// * `ctx` - The application context containing all dependencies
    pub fn new(ctx: &AppContext) -> Self {
        let proxy_manager = ProxyManager::new(ctx);
        Self {
            ctx: ctx.clone(),
            proxy_manager,
        }
    }

    /// Build proxy environment variables
    fn build_proxy_env_vars(&self) -> Vec<String> {
        let proxy_url = self.proxy_manager.proxy_url();
        let mut env = vec![
            format!("HTTP_PROXY={proxy_url}"),
            format!("HTTPS_PROXY={proxy_url}"),
            format!("http_proxy={proxy_url}"),
            format!("https_proxy={proxy_url}"),
            // Include tsk-proxy so HTTP clients bypass Squid when connecting
            // to socat forwarders for host services
            "NO_PROXY=localhost,127.0.0.1,tsk-proxy".to_string(),
            "no_proxy=localhost,127.0.0.1,tsk-proxy".to_string(),
        ];

        // Add host service environment variables if configured
        if self.ctx.tsk_config().has_host_services() {
            env.push(format!(
                "TSK_HOST_SERVICES={}",
                self.ctx.tsk_config().proxy.host_services_env()
            ));
            env.push("TSK_HOST_SERVICES_HOST=tsk-proxy".to_string());
        }

        env
    }

    /// Remove a container with force option
    async fn remove_container(&self, container_id: &str) -> Result<(), String> {
        self.ctx
            .docker_client()
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| e.to_string())
    }

    /// Build bind volumes for container
    fn build_bind_volumes(&self, task: &crate::task::Task, agent: &dyn Agent) -> Vec<String> {
        let repo_path = task
            .copied_repo_path
            .as_ref()
            .expect("Task must have copied_repo_path set before container execution");
        let repo_path_str = repo_path
            .to_str()
            .expect("Repository path should be valid UTF-8");
        let working_dir = container_working_dir(&task.project);
        let mut binds = vec![format!("{repo_path_str}:{working_dir}")];

        // Add agent-specific volumes
        for (host_path, container_path, options) in agent.volumes() {
            let bind = if options.is_empty() {
                format!("{host_path}:{container_path}")
            } else {
                format!("{host_path}:{container_path}:{options}")
            };
            binds.push(bind);
        }

        // Add instructions directory mount
        let instructions_file_path = PathBuf::from(&task.instructions_file);
        if let Some(parent) = instructions_file_path.parent() {
            let abs_parent = parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf());
            binds.push(format!("{}:/instructions:ro", abs_parent.to_string_lossy()));
        }

        // Add output directory mount
        if let Some(task_dir) = repo_path.parent() {
            let output_dir = task_dir.join("output");
            binds.push(format!("{}:/output", output_dir.to_string_lossy()));
        }

        // Add project-specific volume mounts from config
        if let Some(project_config) = self.ctx.tsk_config().get_project_config(&task.project) {
            for volume in &project_config.volumes {
                match volume {
                    VolumeMount::Bind(bind) => {
                        if let Ok(host_path) = bind.expanded_host_path() {
                            let bind_str = if bind.readonly {
                                format!("{}:{}:ro", host_path.display(), bind.container)
                            } else {
                                format!("{}:{}", host_path.display(), bind.container)
                            };
                            binds.push(bind_str);
                        }
                    }
                    VolumeMount::Named(named) => {
                        let volume_name = format!("tsk-{}", named.name);
                        let bind_str = if named.readonly {
                            format!("{volume_name}:{}:ro", named.container)
                        } else {
                            format!("{volume_name}:{}", named.container)
                        };
                        binds.push(bind_str);
                    }
                }
            }
        }

        binds
    }

    /// Generate container name based on task mode
    fn build_container_name(&self, task: &crate::task::Task) -> String {
        if task.is_interactive {
            format!("tsk-interactive-{}", task.id)
        } else {
            format!("tsk-{}", task.id)
        }
    }

    /// Create a container configuration for both interactive and non-interactive modes.
    ///
    /// This function builds a unified container configuration that can be used for
    /// both interactive (TTY-attached) and non-interactive (log-streaming) containers.
    ///
    /// # Arguments
    /// * `image` - The Docker image to use
    /// * `task` - The task containing all necessary configuration
    /// * `agent` - The agent to get volumes and environment variables from
    /// * `network_name` - The Docker network name for the container to join
    ///
    /// # Returns
    /// A `ContainerCreateBody` configured with appropriate settings for the mode
    fn create_container_config(
        &self,
        image: &str,
        task: &crate::task::Task,
        agent: &dyn Agent,
        network_name: Option<&str>,
    ) -> ContainerCreateBody {
        let binds = self.build_bind_volumes(task, agent);
        let instructions_file_path = PathBuf::from(&task.instructions_file);
        let working_dir = container_working_dir(&task.project);

        let mut env_vars = if network_name.is_some() {
            self.build_proxy_env_vars()
        } else {
            Vec::new()
        };

        // Add TSK environment variables for container detection
        env_vars.push("TSK_CONTAINER=1".to_string());
        env_vars.push(format!("TSK_TASK_ID={}", task.id));

        // Add agent-specific environment variables
        for (key, value) in agent.environment() {
            env_vars.push(format!("{key}={value}"));
        }

        // Add project-specific environment variables from config
        if let Some(project_config) = self.ctx.tsk_config().get_project_config(&task.project) {
            for env_var in &project_config.env {
                env_vars.push(format!("{}={}", env_var.name, env_var.value));
            }
        }

        // Set PYTHONPATH for Python stacks so imports work from the project directory
        if task.stack == "python" {
            env_vars.push(format!("PYTHONPATH={working_dir}"));
        }

        let agent_command = agent.build_command(
            instructions_file_path.to_str().unwrap_or("instructions.md"),
            task.is_interactive,
        );

        let command = if agent_command.is_empty() {
            None
        } else {
            Some(agent_command)
        };

        // Load Docker resource limits from user configuration
        let docker_config = &self.ctx.tsk_config().docker;

        let mut cap_drop = vec![
            "NET_ADMIN".to_string(),
            "SETPCAP".to_string(),
            "SYS_ADMIN".to_string(),
            "SYS_PTRACE".to_string(),
            "DAC_OVERRIDE".to_string(),
            "AUDIT_WRITE".to_string(),
            "SETUID".to_string(),
            "SETGID".to_string(),
        ];
        if network_name.is_some() {
            cap_drop.push("NET_RAW".to_string());
        }

        ContainerCreateBody {
            image: Some(image.to_string()),
            // No entrypoint needed anymore - just run as agent user directly
            user: Some(CONTAINER_USER.to_string()),
            cmd: command,
            host_config: Some(HostConfig {
                binds: Some(binds),
                network_mode: network_name.map(|n| n.to_string()),
                memory: Some(docker_config.memory_limit_bytes()),
                cpu_quota: Some(docker_config.cpu_quota_microseconds()),
                cap_drop: Some(cap_drop),
                ..Default::default()
            }),
            working_dir: Some(working_dir),
            env: Some(env_vars),
            attach_stdin: Some(task.is_interactive),
            attach_stdout: Some(task.is_interactive),
            attach_stderr: Some(task.is_interactive),
            tty: Some(task.is_interactive),
            open_stdin: Some(task.is_interactive),
            ..Default::default()
        }
    }

    /// Run a task container with unified support for both interactive and non-interactive modes.
    ///
    /// Follows a setup → execute → cleanup structure:
    /// 1. **Setup**: Conditionally create an isolated network and connect the proxy
    /// 2. **Execute**: Create, configure, and run the container via [`run_container_inner`]
    /// 3. **Cleanup**: Always remove the container, tear down the network, and stop
    ///    the proxy if idle — regardless of whether execution succeeded or failed
    ///
    /// # Arguments
    /// * `docker_image_tag` - Docker image tag to use
    /// * `task` - The task to execute
    /// * `agent` - The agent to use for the task
    ///
    /// # Returns
    /// * `Ok((output, task_result))` - The container output and synthesized task result
    /// * `Err(String)` - Error message if container infrastructure fails
    pub async fn run_task_container(
        &self,
        docker_image_tag: &str,
        task: &crate::task::Task,
        agent: &dyn Agent,
    ) -> Result<(String, crate::agent::TaskResult), String> {
        // --- Setup: conditionally create network and connect proxy ---
        let network_name = if task.network_isolation {
            if let Err(e) = self.proxy_manager.ensure_proxy().await {
                return Err(format!(
                    "Failed to ensure proxy is running and healthy: {e}. \
                    The task should be retried later when the proxy is available. \
                    Check the status in Docker."
                ));
            }

            let name = self
                .proxy_manager
                .create_agent_network(&task.id)
                .await
                .map_err(|e| format!("Failed to create agent network: {e}"))?;

            if let Err(e) = self.proxy_manager.connect_proxy_to_network(&name).await {
                self.proxy_manager.cleanup_agent_network(&name).await;
                return Err(format!("Failed to connect proxy to agent network: {e}"));
            }
            Some(name)
        } else {
            None
        };

        // --- Execute: run the container, capturing its ID for cleanup ---
        let (container_id, result) = self
            .run_container_inner(docker_image_tag, task, agent, network_name.as_deref())
            .await;

        // --- Cleanup: always runs regardless of success/failure ---
        if let Some(ref id) = container_id {
            let _ = self.remove_container(id).await;
        }
        if let Some(ref name) = network_name {
            self.proxy_manager.cleanup_agent_network(name).await;
        }
        if network_name.is_some()
            && let Err(e) = self.proxy_manager.maybe_stop_proxy().await
        {
            eprintln!("Warning: Failed to check/stop idle proxy: {e}");
        }

        result
    }

    /// Execute the container lifecycle without performing resource cleanup.
    ///
    /// Returns the container ID (if one was created) alongside the execution result,
    /// so the caller can clean up the container, network, and proxy in one place.
    async fn run_container_inner(
        &self,
        docker_image_tag: &str,
        task: &crate::task::Task,
        agent: &dyn Agent,
        network_name: Option<&str>,
    ) -> (
        Option<String>,
        Result<(String, crate::agent::TaskResult), String>,
    ) {
        let config = self.create_container_config(docker_image_tag, task, agent, network_name);
        let container_name = self.build_container_name(task);
        let options = bollard::query_parameters::CreateContainerOptionsBuilder::default()
            .name(&container_name)
            .build();

        let container_id = match self
            .ctx
            .docker_client()
            .create_container(Some(options), config)
            .await
        {
            Ok(id) => id,
            Err(e) => return (None, Err(e)),
        };

        // Copy agent files into container before starting
        for (tar_data, dest_path) in agent.files_to_copy() {
            if let Err(e) = self
                .ctx
                .docker_client()
                .upload_to_container(&container_id, &dest_path, tar_data)
                .await
            {
                return (Some(container_id), Err(e));
            }
        }

        if let Err(e) = self
            .ctx
            .docker_client()
            .start_container(&container_id)
            .await
        {
            return (Some(container_id), Err(e));
        }

        let result = if task.is_interactive {
            println!("\nStarting interactive session...");

            match self
                .ctx
                .docker_client()
                .attach_container(&container_id)
                .await
            {
                Ok(()) => {
                    println!("\nInteractive session ended");
                    Ok((
                        String::new(),
                        crate::agent::TaskResult {
                            success: true,
                            message: "Interactive session completed".to_string(),
                            cost_usd: None,
                            duration_ms: None,
                        },
                    ))
                }
                Err(e) => {
                    eprintln!("Interactive session ended with error: {e}");
                    Err(e)
                }
            }
        } else {
            println!("Starting agent sand box container: {container_id}");

            let mut log_processor = agent.create_log_processor(Some(task));
            let output = self
                .stream_container_logs(&container_id, &mut *log_processor)
                .await;
            let task_result = log_processor.get_final_result().cloned();

            match output {
                Ok((output, exit_code)) => {
                    let task_result = match (exit_code, task_result) {
                        // Non-zero exit code: always failure
                        (code, Some(mut r)) if code != 0 => {
                            r.success = false;
                            r
                        }
                        (code, None) if code != 0 => crate::agent::TaskResult {
                            success: false,
                            message: format!("Container exited with status {code}"),
                            cost_usd: None,
                            duration_ms: None,
                        },
                        // Zero exit code: use agent result if available
                        (_, Some(r)) => r,
                        // Zero exit code, no agent result: success
                        (_, None) => crate::agent::TaskResult {
                            success: true,
                            message: "Task completed".to_string(),
                            cost_usd: None,
                            duration_ms: None,
                        },
                    };
                    Ok((output, task_result))
                }
                Err(e) => Err(e),
            }
        };

        (Some(container_id), result)
    }

    /// Stream container logs and process them through the log processor.
    ///
    /// Returns the accumulated log output and the container's exit code.
    /// `Err` is reserved for Docker API / infrastructure failures only.
    async fn stream_container_logs(
        &self,
        container_id: &str,
        log_processor: &mut dyn LogProcessor,
    ) -> Result<(String, i64), String> {
        // Start a background task to stream logs
        let client_clone = self.ctx.docker_client();
        let container_id_clone = container_id.to_string();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);

        let log_task = tokio::spawn(async move {
            let log_options = LogsOptions {
                stdout: true,
                stderr: true,
                follow: true,
                timestamps: false,
                ..Default::default()
            };

            match client_clone
                .logs_stream(&container_id_clone, Some(log_options))
                .await
            {
                Ok(mut stream) => {
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(log_line) => {
                                if tx.send(log_line).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                eprintln!("Error streaming logs: {e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to start log streaming: {e}");
                }
            }
        });

        // Collect all logs for return value
        let mut all_logs = String::new();

        // Buffer for accumulating partial lines from Docker chunks
        let mut line_buffer = String::new();

        // Get docker client to avoid temporary value issues
        let docker_client = self.ctx.docker_client();

        // Create the wait future ONCE and pin it so it persists across loop iterations.
        // This is critical - tokio::select! in a loop drops unselected futures each iteration,
        // so without pinning, wait_container would be called multiple times.
        let wait_future = docker_client.wait_container(container_id);
        tokio::pin!(wait_future);

        // Process logs while container is running
        loop {
            tokio::select! {
                Some(log_chunk) = rx.recv() => {
                    // Keep raw chunks in all_logs for full log capture
                    all_logs.push_str(&log_chunk);

                    // Buffer chunks and process complete lines only
                    line_buffer.push_str(&log_chunk);
                    process_complete_lines(&mut line_buffer, log_processor);
                }
                exit_code = &mut wait_future => {
                    let exit_code = exit_code?;

                    // Give a bit of time for remaining logs
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                    // Drain any remaining logs
                    while let Ok(log_chunk) = rx.try_recv() {
                        all_logs.push_str(&log_chunk);
                        line_buffer.push_str(&log_chunk);
                        process_complete_lines(&mut line_buffer, log_processor);
                    }

                    // Flush remaining buffer content if non-empty
                    if !line_buffer.trim().is_empty() {
                        let trimmed = line_buffer.trim_end_matches('\r');
                        if let Some(formatted) = log_processor.process_line(trimmed) {
                            println!("{formatted}");
                        }
                    }

                    // Abort the log task
                    log_task.abort();

                    return Ok((all_logs, exit_code));
                }
            }
        }
    }
}

/// Process complete lines from the buffer and pass them to the log processor.
///
/// This function extracts all complete lines (terminated by newline) from the buffer,
/// processes each through the log processor, and removes them from the buffer.
/// Any partial line (without a trailing newline) remains in the buffer.
fn process_complete_lines(line_buffer: &mut String, log_processor: &mut dyn LogProcessor) {
    while let Some(newline_pos) = line_buffer.find('\n') {
        let complete_line = &line_buffer[..newline_pos];
        // Handle CRLF by trimming trailing \r
        let trimmed = complete_line.trim_end_matches('\r');

        if let Some(formatted) = log_processor.process_line(trimmed) {
            println!("{formatted}");
        }

        // Use drain() for efficient in-place removal
        line_buffer.drain(..=newline_pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::task::{Task, TaskStatus};
    use crate::test_utils::TrackedDockerClient;
    use std::sync::Arc;

    fn create_test_task(is_interactive: bool) -> Task {
        let repo_path = PathBuf::from("/tmp/test-repo");
        Task {
            id: "test-task-id".to_string(),
            repo_root: repo_path.clone(),
            task_type: "feature".to_string(),
            instructions_file: "/tmp/test-repo/.tsk/tasks/instructions.md".to_string(),
            status: TaskStatus::Running,
            started_at: Some(chrono::Utc::now()),
            branch_name: "tsk/feature/test-task/test-task-id".to_string(),
            copied_repo_path: Some(repo_path),
            is_interactive,
            ..Task::test_default()
        }
    }

    #[tokio::test]
    async fn test_run_task_container_success() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());
        let (output, task_result) = result.unwrap();
        assert_eq!(output, "Container logs");
        assert!(task_result.success);
        assert_eq!(task_result.message, "Task completed");

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check that the command includes the agent command
        let task_container_config = &create_calls[1].1;
        let actual_cmd = task_container_config.cmd.as_ref().unwrap();
        // Command should be sh -c with the agent command
        assert_eq!(actual_cmd.len(), 3);
        assert_eq!(actual_cmd[0], "sh");
        assert_eq!(actual_cmd[1], "-c");
        assert!(actual_cmd[2].contains("claude"));

        // Check that user is set
        assert_eq!(task_container_config.user, Some("agent".to_string()));

        // Check proxy environment variables
        let env = task_container_config.env.as_ref().unwrap();
        assert!(env.contains(&"HTTP_PROXY=http://tsk-proxy:3128".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=http://tsk-proxy:3128".to_string()));

        // Check TSK environment variables
        assert!(env.contains(&"TSK_CONTAINER=1".to_string()));
        assert!(env.contains(&"TSK_TASK_ID=test-task-id".to_string()));
        drop(create_calls); // Release the lock

        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 2); // One for proxy, one for task container
        assert_eq!(start_calls[0], "tsk-proxy");
        assert_eq!(start_calls[1], "test-container-id-1");

        let wait_calls = mock_client.wait_container_calls.lock().unwrap();
        assert_eq!(wait_calls.len(), 1);
        assert_eq!(wait_calls[0], "test-container-id-1");

        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, "test-container-id-1");
        drop(remove_calls);

        // Verify upload_to_container was called for agent files
        // Note: In tests, claude.json may or may not exist, so we just verify it was called
        // if the agent has files to copy
        let upload_calls = mock_client.upload_to_container_calls.lock().unwrap();
        // The number of calls depends on whether .claude.json exists in test environment
        // For this test, just verify the method was callable
        for (container_id, dest_path, _tar_data) in upload_calls.iter() {
            assert_eq!(container_id, "test-container-id-1");
            assert_eq!(dest_path, "/home/agent");
        }
    }

    #[tokio::test]
    async fn test_run_task_container_interactive() {
        // Test interactive mode with mock client
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(true);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        // Interactive mode should succeed with mock client
        assert!(result.is_ok());
        let (output, task_result) = result.unwrap();
        // Interactive mode returns empty output and a success task result
        assert_eq!(output, "");
        assert!(task_result.success);
        assert_eq!(task_result.message, "Interactive session completed");

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check task container config for interactive mode
        let (options, config) = &create_calls[1];
        assert_eq!(
            options.as_ref().unwrap().name,
            Some("tsk-interactive-test-task-id".to_string())
        );

        // Check interactive-specific settings
        assert_eq!(config.attach_stdin, Some(true));
        assert_eq!(config.attach_stdout, Some(true));
        assert_eq!(config.attach_stderr, Some(true));
        assert_eq!(config.tty, Some(true));
        assert_eq!(config.open_stdin, Some(true));

        // Check TSK environment variables are present in interactive mode too
        let env = config.env.as_ref().unwrap();
        assert!(env.contains(&"TSK_CONTAINER=1".to_string()));
        assert!(env.contains(&"TSK_TASK_ID=test-task-id".to_string()));

        // Verify attach_container was called (indirectly through start_container_calls)
        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 2); // One for proxy, one for task container

        // Verify cleanup happened
        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1); // Task container should be removed
        assert_eq!(remove_calls[0].0, "test-container-id-1");
    }

    #[tokio::test]
    async fn test_run_task_container_non_zero_exit() {
        // Set up a mock client that will return a non-zero exit code
        let mock_client = Arc::new(TrackedDockerClient {
            exit_code: 1,
            ..Default::default()
        });
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());

        // Run the task container
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        // Non-zero exit is not an infrastructure error; it returns Ok with a failed TaskResult
        assert!(result.is_ok());
        let (output, task_result) = result.unwrap();
        assert!(!task_result.success);
        assert!(
            task_result
                .message
                .contains("Container exited with status 1")
        );
        assert!(output.contains("Container logs"));

        // Cleanup still happens
        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, "test-container-id-1");
        drop(remove_calls);

        // Network cleanup should also happen
        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 1);
        drop(disconnect_calls);

        let remove_network_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_network_calls.len(), 1);
    }

    #[tokio::test]
    async fn test_run_task_container_network_setup_fails() {
        let mock_client = TrackedDockerClient {
            network_exists: false,
            create_network_error: Some("Docker daemon not running".to_string()),
            ..Default::default()
        };
        let mock_client = Arc::new(mock_client);
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        // The error message should indicate proxy startup failure
        assert!(error_msg.contains("Failed to ensure proxy is running and healthy"));
        // The error chain should include the network creation failure
        assert!(
            error_msg.contains("Failed to ensure network exists")
                || error_msg.contains("Failed to create network")
        );

        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_container_configuration() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let _ = manager.run_task_container("tsk/base", &task, &agent).await;

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check proxy container config (proxy manager creates this)
        let (proxy_options, _proxy_config) = &create_calls[0];
        assert_eq!(
            proxy_options.as_ref().unwrap().name,
            Some("tsk-proxy".to_string())
        );

        // Check task container config
        let (options, config) = &create_calls[1];

        assert!(
            options
                .as_ref()
                .unwrap()
                .name
                .as_ref()
                .unwrap()
                .starts_with("tsk-")
        );
        assert_eq!(
            options.as_ref().unwrap().name,
            Some("tsk-test-task-id".to_string())
        );
        assert_eq!(config.image, Some("tsk/base".to_string()));
        assert_eq!(config.working_dir, Some("/workspace/default".to_string()));
        // User is now set directly
        assert_eq!(config.user, Some(CONTAINER_USER.to_string()));

        // Check that command includes the agent command
        let actual_cmd = config.cmd.as_ref().unwrap();
        assert_eq!(actual_cmd.len(), 3);
        assert_eq!(actual_cmd[0], "sh");
        assert_eq!(actual_cmd[1], "-c");
        assert!(actual_cmd[2].contains("claude"));

        // No entrypoint anymore
        assert!(config.entrypoint.is_none());

        let host_config = config.host_config.as_ref().unwrap();
        // Network mode should use task-specific isolated network
        assert_eq!(
            host_config.network_mode,
            Some("tsk-agent-test-task-id".to_string())
        );
        // Agent containers should NOT have extra_hosts (no direct host access)
        assert!(host_config.extra_hosts.is_none());
        let default_options = crate::context::DockerOptions::default();
        assert_eq!(
            host_config.memory,
            Some(default_options.memory_limit_bytes())
        );
        assert_eq!(
            host_config.cpu_quota,
            Some(default_options.cpu_quota_microseconds())
        );

        let binds = host_config.binds.as_ref().unwrap();
        assert_eq!(binds.len(), 4); // workspace, claude dir, instructions, and output
        assert!(binds[0].contains("/tmp/test-repo:/workspace/default"));
        // In test mode, .claude directory is in temp directory
        assert!(binds[1].contains(":/home/agent/.claude"));
        assert!(binds[2].contains(":/instructions:ro"));
        assert!(binds[3].contains(":/output"));

        // Check proxy environment variables
        let env = config.env.as_ref().unwrap();
        assert!(env.contains(&"HTTP_PROXY=http://tsk-proxy:3128".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=http://tsk-proxy:3128".to_string()));
        assert!(env.contains(&"NO_PROXY=localhost,127.0.0.1,tsk-proxy".to_string()));
        assert!(env.contains(&"no_proxy=localhost,127.0.0.1,tsk-proxy".to_string()));
        drop(create_calls);

        // Verify network lifecycle operations were called
        let create_internal_network_calls =
            mock_client.create_internal_network_calls.lock().unwrap();
        assert_eq!(create_internal_network_calls.len(), 1);
        assert_eq!(create_internal_network_calls[0], "tsk-agent-test-task-id");
        drop(create_internal_network_calls);

        let connect_calls = mock_client.connect_network_calls.lock().unwrap();
        assert_eq!(connect_calls.len(), 1);
        assert_eq!(
            connect_calls[0],
            (
                "tsk-proxy".to_string(),
                "tsk-agent-test-task-id".to_string()
            )
        );
        drop(connect_calls);

        // Verify network cleanup was called
        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 1);
        assert_eq!(
            disconnect_calls[0],
            (
                "tsk-proxy".to_string(),
                "tsk-agent-test-task-id".to_string()
            )
        );
        drop(disconnect_calls);

        let remove_network_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_network_calls.len(), 1);
        assert_eq!(remove_network_calls[0], "tsk-agent-test-task-id");
    }

    #[tokio::test]
    async fn test_run_task_container_with_instructions_file() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let mut task = create_test_task(false);
        task.instructions_file = "/tmp/tsk-test/instructions.txt".to_string();
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check that instructions directory is mounted
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();
        assert_eq!(binds.len(), 4); // workspace, claude dir, instructions, and output
        assert!(binds[2].contains("/tmp/tsk-test:/instructions:ro"));
        assert!(binds[3].contains(":/output"));
    }

    #[tokio::test]
    async fn test_relative_path_conversion() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        // Create a temporary directory to use as base
        let temp_dir = tempfile::TempDir::new().unwrap();
        let absolute_path = temp_dir.path().join("test-repo");

        let mut task = create_test_task(false);
        task.copied_repo_path = Some(absolute_path.clone());
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container
        let (_, config) = &create_calls[1]; // Get the task container config, not proxy

        let host_config = config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();
        let repo_bind = &binds[0];

        // Should contain an absolute path (starts with /)
        assert!(repo_bind.starts_with('/'));
        assert!(repo_bind.contains("test-repo"));
        assert!(repo_bind.ends_with(":/workspace/default"));

        // Should also have the claude directory, instructions, and output mounts
        assert_eq!(binds.len(), 4); // workspace, claude dir, instructions, and output
        // In test mode, .claude directory is in temp directory
        assert!(binds[1].contains(":/home/agent/.claude"));
        assert!(binds[2].contains(":/instructions:ro"));
        assert!(binds[3].contains(":/output"));
    }

    #[tokio::test]
    async fn test_project_volume_mounts_bind() {
        use crate::context::{BindMount, ProjectConfig, TskConfig, VolumeMount};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with a bind mount for the test project
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            ProjectConfig {
                agent: None,
                stack: None,
                volumes: vec![VolumeMount::Bind(BindMount {
                    host: "/host/cache".to_string(),
                    container: "/container/cache".to_string(),
                    readonly: false,
                })],
                env: vec![],
            },
        );
        let tsk_config = TskConfig {
            project: project_configs,
            ..Default::default()
        };

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .with_tsk_config(tsk_config)
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();

        // Should have base binds + project bind mount
        assert_eq!(binds.len(), 5);
        assert!(
            binds
                .iter()
                .any(|b| b.contains("/host/cache:/container/cache"))
        );
    }

    #[tokio::test]
    async fn test_project_volume_mounts_named() {
        use crate::context::{NamedVolume, ProjectConfig, TskConfig, VolumeMount};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with a named volume for the test project
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            ProjectConfig {
                agent: None,
                stack: None,
                volumes: vec![VolumeMount::Named(NamedVolume {
                    name: "build-cache".to_string(),
                    container: "/container/cache".to_string(),
                    readonly: false,
                })],
                env: vec![],
            },
        );
        let tsk_config = TskConfig {
            project: project_configs,
            ..Default::default()
        };

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .with_tsk_config(tsk_config)
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();

        // Should have base binds + named volume (prefixed with tsk-)
        assert_eq!(binds.len(), 5);
        assert!(
            binds
                .iter()
                .any(|b| b.contains("tsk-build-cache:/container/cache"))
        );
    }

    #[tokio::test]
    async fn test_project_volume_mounts_readonly() {
        use crate::context::{BindMount, ProjectConfig, TskConfig, VolumeMount};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with a readonly bind mount
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            ProjectConfig {
                agent: None,
                stack: None,
                volumes: vec![VolumeMount::Bind(BindMount {
                    host: "/etc/ssl/certs".to_string(),
                    container: "/etc/ssl/certs".to_string(),
                    readonly: true,
                })],
                env: vec![],
            },
        );
        let tsk_config = TskConfig {
            project: project_configs,
            ..Default::default()
        };

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .with_tsk_config(tsk_config)
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();

        // Should have base binds + readonly bind mount
        assert_eq!(binds.len(), 5);
        assert!(
            binds
                .iter()
                .any(|b| b.contains("/etc/ssl/certs:/etc/ssl/certs:ro"))
        );
    }

    #[tokio::test]
    async fn test_no_project_volumes_when_project_not_configured() {
        // Test that binds don't include project volumes when project is not in config
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let mut task = create_test_task(false);
        task.project = "unconfigured-project".to_string();
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let host_config = task_container_config.host_config.as_ref().unwrap();
        let binds = host_config.binds.as_ref().unwrap();

        // Should only have base binds: workspace, claude dir, instructions, output
        assert_eq!(binds.len(), 4);
    }

    #[tokio::test]
    async fn test_project_env_vars() {
        use crate::context::{EnvVar, ProjectConfig, TskConfig};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with environment variables for the test project
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            ProjectConfig {
                agent: None,
                stack: None,
                volumes: vec![],
                env: vec![
                    EnvVar {
                        name: "DATABASE_URL".to_string(),
                        value: "postgres://tsk-proxy:5432/mydb".to_string(),
                    },
                    EnvVar {
                        name: "DEBUG".to_string(),
                        value: "true".to_string(),
                    },
                ],
            },
        );
        let tsk_config = TskConfig {
            project: project_configs,
            ..Default::default()
        };

        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .with_tsk_config(tsk_config)
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let env = task_container_config.env.as_ref().unwrap();

        // Should have project env vars added
        assert!(
            env.iter()
                .any(|e| e == "DATABASE_URL=postgres://tsk-proxy:5432/mydb")
        );
        assert!(env.iter().any(|e| e == "DEBUG=true"));
    }

    #[tokio::test]
    async fn test_python_stack_sets_pythonpath() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let mut task = create_test_task(false);
        task.stack = "python".to_string();
        task.project = "my-python-app".to_string();
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let _ = manager.run_task_container("tsk/base", &task, &agent).await;

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let env = task_container_config.env.as_ref().unwrap();
        assert!(env.contains(&"PYTHONPATH=/workspace/my-python-app".to_string()));

        // Also verify working_dir uses the project name
        assert_eq!(
            task_container_config.working_dir,
            Some("/workspace/my-python-app".to_string())
        );
    }

    #[tokio::test]
    async fn test_non_python_stack_no_pythonpath() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false); // stack is "default"
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let _ = manager.run_task_container("tsk/base", &task, &agent).await;

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let env = task_container_config.env.as_ref().unwrap();
        assert!(!env.iter().any(|e| e.starts_with("PYTHONPATH=")));
    }

    #[tokio::test]
    async fn test_no_project_env_vars_when_project_not_configured() {
        // Test that env vars don't include project env vars when project is not in config
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let mut task = create_test_task(false);
        task.project = "unconfigured-project".to_string();
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_container_config = &create_calls[1].1;
        let env = task_container_config.env.as_ref().unwrap();

        // Should not have any project-specific env vars (only proxy and agent env vars)
        assert!(
            !env.iter()
                .any(|e| e.starts_with("DATABASE_URL=") || e.starts_with("DEBUG="))
        );
    }

    #[tokio::test]
    async fn test_run_task_container_no_network_isolation() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let mut task = create_test_task(false);
        task.network_isolation = false;
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_ok());

        // Verify NO proxy or network operations occurred
        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 1); // Only task container, no proxy

        let task_config = &create_calls[0].1;

        // No proxy env vars
        let env = task_config.env.as_ref().unwrap();
        assert!(!env.iter().any(|e| e.starts_with("HTTP_PROXY=")));
        assert!(!env.iter().any(|e| e.starts_with("HTTPS_PROXY=")));
        assert!(!env.iter().any(|e| e.starts_with("NO_PROXY=")));
        assert!(!env.iter().any(|e| e.starts_with("no_proxy=")));

        // TSK env vars should still be present
        assert!(env.contains(&"TSK_CONTAINER=1".to_string()));
        assert!(env.contains(&"TSK_TASK_ID=test-task-id".to_string()));

        // network_mode should be None
        let host_config = task_config.host_config.as_ref().unwrap();
        assert!(
            host_config.network_mode.is_none(),
            "network_mode should be None when isolation is disabled"
        );

        // NET_RAW should NOT be in cap_drop
        let cap_drop = host_config.cap_drop.as_ref().unwrap();
        assert!(
            !cap_drop.contains(&"NET_RAW".to_string()),
            "NET_RAW should not be dropped"
        );
        // But NET_ADMIN should still be dropped
        assert!(
            cap_drop.contains(&"NET_ADMIN".to_string()),
            "NET_ADMIN should still be dropped"
        );
        drop(create_calls);

        // No network operations
        let create_network_calls = mock_client.create_internal_network_calls.lock().unwrap();
        assert_eq!(create_network_calls.len(), 0);
        drop(create_network_calls);

        let connect_calls = mock_client.connect_network_calls.lock().unwrap();
        assert_eq!(connect_calls.len(), 0);
        drop(connect_calls);

        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 0);
        drop(disconnect_calls);

        let remove_network_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_network_calls.len(), 0);
    }

    #[tokio::test]
    async fn test_cleanup_on_container_create_failure() {
        let mock_client = Arc::new(TrackedDockerClient {
            create_container_error: Some("out of disk space".to_string()),
            ..Default::default()
        });
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("out of disk space"));

        // No container was created, so no remove_container call
        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 0);
        drop(remove_calls);

        // Network cleanup should still happen (setup succeeded before container create failed)
        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 1);
        drop(disconnect_calls);

        let remove_network_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_network_calls.len(), 1);
    }

    #[tokio::test]
    async fn test_cleanup_on_start_container_failure() {
        let mock_client = Arc::new(TrackedDockerClient {
            start_container_error: Some("container runtime error".to_string()),
            ..Default::default()
        });
        let ctx = AppContext::builder()
            .with_docker_client(mock_client.clone())
            .build();
        let manager = DockerManager::new(&ctx);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let result = manager.run_task_container("tsk/base", &task, &agent).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("container runtime error"));

        // Container was created, so cleanup should remove it
        let remove_calls = mock_client.remove_container_calls.lock().unwrap();
        assert_eq!(remove_calls.len(), 1);
        assert_eq!(remove_calls[0].0, "test-container-id-1");
        drop(remove_calls);

        // Network cleanup should happen
        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 1);
        drop(disconnect_calls);

        let remove_network_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_network_calls.len(), 1);
    }
}
