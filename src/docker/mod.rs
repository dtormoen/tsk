pub mod build_lock_manager;
pub mod composer;
pub mod image_manager;
pub mod layers;
pub mod proxy_manager;
pub mod template_engine;
use crate::agent::{Agent, LogProcessor};
use crate::context::AppContext;
use crate::context::ContainerEngine;
use crate::context::VolumeMount;
use crate::context::docker_client::DockerClient;
use crate::context::tsk_config;
use crate::docker::proxy_manager::ProxyManager;
use crate::tui::events::{ServerEvent, ServerEventSender};
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{LogsOptions, RemoveContainerOptions};
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

const CONTAINER_WORKSPACE_BASE: &str = "/workspace";
const CONTAINER_USER: &str = "agent";
const SECCOMP_DIND_PROFILE: &str = include_str!("seccomp_dind.json");

/// Standard proxy environment variable names forwarded to/from containers.
/// Additional variables like JAVA_TOOL_OPTIONS and TSK_PROXY_HOST are handled separately.
pub(crate) const PROXY_ENV_VARS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "http_proxy",
    "https_proxy",
    "NO_PROXY",
    "no_proxy",
];

fn container_working_dir(project: &str) -> String {
    format!("{CONTAINER_WORKSPACE_BASE}/{project}")
}

/// Resolve config for a task, preferring the DB snapshot over live resolution.
///
/// If the task has a `resolved_config` snapshot (set at creation time), deserialize it.
/// Otherwise, fall back to live resolution from config files (for pre-migration tasks).
pub(crate) fn resolve_config_from_task(
    task: &crate::task::Task,
    ctx: &AppContext,
    event_sender: &Option<crate::tui::events::ServerEventSender>,
) -> crate::context::ResolvedConfig {
    if let Some(ref json) = task.resolved_config {
        match serde_json::from_str(json) {
            Ok(config) => return config,
            Err(e) => {
                crate::tui::events::emit_or_print(
                    event_sender,
                    crate::tui::events::ServerEvent::WarningMessage(format!(
                        "Warning: Failed to deserialize resolved_config for task {}: {e}. Falling back to live resolution.",
                        task.id
                    )),
                );
            }
        }
    }
    // Fallback: live resolution (for tasks created before config snapshotting)
    let project_config = tsk_config::load_project_config(&task.repo_root);
    ctx.tsk_config().resolve_config(
        &task.project,
        project_config.as_ref(),
        Some(&task.repo_root),
    )
}

/// Write build output to a log file on failure, printing warnings on error.
pub(crate) fn save_build_log(
    log_path: &std::path::Path,
    build_output: &str,
    event_sender: &Option<crate::tui::events::ServerEventSender>,
) {
    use crate::tui::events::{ServerEvent, emit_or_print};
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(write_err) = std::fs::write(log_path, build_output) {
        emit_or_print(
            event_sender,
            ServerEvent::WarningMessage(format!(
                "Warning: Failed to write build log to {}: {write_err}",
                log_path.display()
            )),
        );
    } else {
        emit_or_print(
            event_sender,
            ServerEvent::StatusMessage(format!("Build log saved to: {}", log_path.display())),
        );
    }
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
    client: Arc<dyn DockerClient>,
    proxy_manager: ProxyManager,
    event_sender: Option<ServerEventSender>,
}

impl DockerManager {
    /// Creates a new DockerManager with the given application context and Docker client.
    ///
    /// # Arguments
    /// * `ctx` - The application context containing all dependencies
    /// * `client` - The Docker client for container operations
    /// * `event_sender` - Optional TUI event channel for structured output
    pub fn new(
        ctx: &AppContext,
        client: Arc<dyn DockerClient>,
        event_sender: Option<ServerEventSender>,
    ) -> Self {
        let proxy_manager = ProxyManager::new(
            client.clone(),
            ctx.tsk_env(),
            ctx.tsk_config().container_engine.clone(),
            event_sender.clone(),
        );
        Self {
            ctx: ctx.clone(),
            client,
            proxy_manager,
            event_sender,
        }
    }

    /// Route an event through the TUI channel when available, otherwise print directly.
    fn emit(&self, event: ServerEvent) {
        crate::tui::events::emit_or_print(&self.event_sender, event);
    }

    /// Returns the Docker client for use by other components
    pub fn client(&self) -> Arc<dyn DockerClient> {
        Arc::clone(&self.client)
    }

    /// Returns true when TSK is running inside a TSK container with proxy
    /// env vars pointing to `tsk-proxy`.
    ///
    /// In this case, proxy and network isolation are handled by the outer
    /// container's Docker environment, so we skip them for nested containers.
    /// We require proxy env vars to reference `tsk-proxy` to avoid silently
    /// granting unrestricted internet access when `TSK_CONTAINER=1` is set
    /// without a properly configured proxy.
    fn is_nested(&self) -> bool {
        // In tests, always use the normal proxy/network path so mocks work correctly
        if cfg!(test) {
            return false;
        }
        std::env::var("TSK_CONTAINER").is_ok() && Self::has_tsk_proxy_env()
    }

    /// Returns true if at least one proxy env var references the `tsk-proxy` host.
    fn has_tsk_proxy_env() -> bool {
        PROXY_ENV_VARS.iter().any(|var| {
            std::env::var(var)
                .map(|val| val.contains("tsk-proxy"))
                .unwrap_or(false)
        })
    }

    /// Build proxy environment variables for the container.
    ///
    /// When nested inside a TSK container, forwards env vars from the outer container.
    /// Otherwise, sets proxy env vars using the proxy config's container name and URL.
    fn build_proxy_env_vars(
        &self,
        resolved: &crate::context::ResolvedConfig,
        proxy_config: &crate::context::ResolvedProxyConfig,
    ) -> Vec<String> {
        if self.is_nested() {
            // Forward proxy env vars from the outer container's environment.
            // The outer TSK container already has network isolation via Docker.
            let mut env = Vec::new();
            for var in PROXY_ENV_VARS
                .iter()
                .copied()
                .chain(["JAVA_TOOL_OPTIONS", "TSK_PROXY_HOST"])
            {
                if let Ok(val) = std::env::var(var) {
                    env.push(format!("{var}={val}"));
                }
            }
            return env;
        }

        let proxy_url = proxy_config.proxy_url();
        let proxy_container_name = proxy_config.proxy_container_name();
        let mut env = vec![
            format!("HTTP_PROXY={proxy_url}"),
            format!("HTTPS_PROXY={proxy_url}"),
            format!("http_proxy={proxy_url}"),
            format!("https_proxy={proxy_url}"),
            format!("NO_PROXY=localhost,127.0.0.1,{proxy_container_name}"),
            format!("no_proxy=localhost,127.0.0.1,{proxy_container_name}"),
        ];

        // JVM proxy system properties via JAVA_TOOL_OPTIONS
        // Maven and Gradle ignore HTTP_PROXY env vars, so this ensures all JVM
        // processes route through the proxy. Harmless for non-Java containers.
        env.push(format!(
            "JAVA_TOOL_OPTIONS=-Dhttp.proxyHost={pcn} -Dhttp.proxyPort=3128 \
             -Dhttps.proxyHost={pcn} -Dhttps.proxyPort=3128 \
             -Dhttp.nonProxyHosts=localhost|127.0.0.1 \
             -Dhttps.nonProxyHosts=localhost|127.0.0.1",
            pcn = proxy_container_name
        ));

        // Always export the proxy container name so scripts (e.g. network
        // isolation tests) can reach the proxy regardless of host_ports config.
        env.push(format!("TSK_PROXY_HOST={proxy_container_name}"));

        // Add host port environment variables if configured
        if resolved.has_host_ports() {
            env.push(format!("TSK_HOST_PORTS={}", resolved.host_ports_env()));
        }

        env
    }

    /// Remove a container with force option
    async fn remove_container(&self, container_id: &str) -> Result<(), String> {
        self.client
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
    fn build_bind_volumes(
        &self,
        task: &crate::task::Task,
        agent: &dyn Agent,
        resolved: &crate::context::ResolvedConfig,
    ) -> Vec<String> {
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

        // Add volume mounts from resolved config (already merged from defaults + project)
        for volume in &resolved.volumes {
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
    /// # Arguments
    /// * `image` - The Docker image to use
    /// * `task` - The task containing all necessary configuration
    /// * `agent` - The agent to get volumes and environment variables from
    /// * `network_name` - The Docker network name for the container to join
    /// * `proxy_config` - The proxy configuration for env var setup
    fn create_container_config(
        &self,
        image: &str,
        task: &crate::task::Task,
        agent: &dyn Agent,
        network_name: Option<&str>,
        proxy_config: Option<&crate::context::ResolvedProxyConfig>,
        proxy_container_ip: Option<&str>,
    ) -> ContainerCreateBody {
        let resolved = resolve_config_from_task(task, &self.ctx, &self.event_sender);
        let binds = self.build_bind_volumes(task, agent, &resolved);
        let instructions_file_path = PathBuf::from(&task.instructions_file);
        let working_dir = container_working_dir(&task.project);

        let mut env_vars = if let Some(pc) = proxy_config {
            self.build_proxy_env_vars(&resolved, pc)
        } else if self.is_nested() {
            // Nested mode: use a default proxy config to forward env vars
            let default_pc = crate::context::ResolvedProxyConfig::default();
            self.build_proxy_env_vars(&resolved, &default_pc)
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

        // Add environment variables from resolved config (already merged from defaults + project)
        for env_var in &resolved.env {
            env_vars.push(format!("{}={}", env_var.name, env_var.value));
        }

        // Use chroot isolation for Podman/Buildah builds inside DIND containers.
        // Full OCI isolation fails in nested user namespaces because the kernel
        // denies devpts mounts (Permission denied). Chroot isolation avoids
        // creating new namespaces or mounting devpts/proc/sysfs during RUN steps,
        // which is safe since we're already inside a container.
        if task.dind {
            env_vars.push("BUILDAH_ISOLATION=chroot".to_string());
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

        let container_engine = &self.ctx.tsk_config().container_engine;

        // DIND security relaxations (seccomp profile, AppArmor, SETUID/SETGID) are opt-in
        let security_opt = if task.dind {
            let mut security_opts = if *container_engine == ContainerEngine::Podman {
                let seccomp_path = self.ctx.tsk_env().config_dir().join("seccomp_dind.json");
                if std::fs::write(&seccomp_path, SECCOMP_DIND_PROFILE).is_ok() {
                    vec![format!("seccomp={}", seccomp_path.display())]
                } else {
                    vec!["seccomp=unconfined".to_string()]
                }
            } else {
                vec![format!("seccomp={SECCOMP_DIND_PROFILE}")]
            };
            security_opts.push("apparmor=unconfined".to_string());
            // Podman-in-Podman: disable SELinux confinement so nested Podman
            // can fchown stdio file descriptors. Without this, SELinux's
            // container_t context blocks fchown with EACCES, which crun treats
            // as fatal (unlike EPERM which crun ignores). Not needed for
            // Docker-in-Podman since Docker does not apply SELinux container_t.
            if *container_engine == ContainerEngine::Podman {
                security_opts.push("label=disable".to_string());
            }
            Some(security_opts)
        } else {
            None
        };

        let mut cap_drop = vec![
            "NET_ADMIN".to_string(),
            "SETPCAP".to_string(),
            "SYS_ADMIN".to_string(),
            "SYS_PTRACE".to_string(),
            "DAC_OVERRIDE".to_string(),
            "AUDIT_WRITE".to_string(),
        ];
        if !task.dind {
            cap_drop.push("SETUID".to_string());
            cap_drop.push("SETGID".to_string());
        }
        if network_name.is_some() {
            cap_drop.push("NET_RAW".to_string());
        }

        // Check if a user-configured volume already covers Podman's storage path
        // before `binds` is moved into the container config.
        let podman_storage = "/home/agent/.local/share/containers/storage";
        let has_storage_volume = binds
            .iter()
            .any(|b| b.split(':').nth(1) == Some(podman_storage));

        ContainerCreateBody {
            image: Some(image.to_string()),
            // No entrypoint needed anymore - just run as agent user directly
            user: Some(CONTAINER_USER.to_string()),
            cmd: command,
            host_config: Some(HostConfig {
                binds: Some(binds),
                // In nested containers, use host networking so the inner container
                // shares the outer container's network namespace and can resolve
                // `tsk-proxy` via Docker's embedded DNS. Without this, Podman's
                // `netns = "none"` in containers.conf gives the inner container
                // no network, causing DNS failures and proxy connection errors.
                network_mode: if self.is_nested() {
                    Some("host".to_string())
                } else {
                    network_name.map(|n| n.to_string())
                },
                memory: if self.is_nested() {
                    None
                } else {
                    Some(resolved.memory_limit_bytes())
                },
                cpu_quota: if self.is_nested() {
                    None
                } else {
                    Some(resolved.cpu_quota_microseconds())
                },
                cap_drop: Some(cap_drop),
                security_opt,
                // WORKAROUND: Mount tmpfs at Podman's storage path so it uses native
                // kernel overlay instead of fuse-overlayfs. Without this, execve()
                // returns EINVAL for any binary on a FUSE mount inside a user
                // namespace — a kernel bug in LinuxKit 6.10.14+ (Docker Desktop
                // 4.33.0+). This breaks all `podman build` and `podman run` inside
                // TSK containers. Native overlay on tmpfs avoids the broken code
                // path entirely. Remove this once docker/for-mac#7413 is fixed.
                //
                // Skip the tmpfs when a user-configured volume already targets this
                // path (e.g. a named volume for caching DIND image layers across runs).
                // The named volume provides the same native overlay benefit.
                tmpfs: if has_storage_volume {
                    None
                } else {
                    Some(HashMap::from([(
                        podman_storage.to_string(),
                        "size=40G".to_string(),
                    )]))
                },
                // In nested containers, keep the host UID mapping so bind-mounted
                // files retain correct ownership (rootless Podman remaps UIDs otherwise).
                userns_mode: if self.is_nested() {
                    Some("keep-id".to_string())
                } else {
                    None
                },
                extra_hosts: match (proxy_config, proxy_container_ip) {
                    (Some(pc), Some(ip)) => {
                        Some(vec![format!("{}:{}", pc.proxy_container_name(), ip)])
                    }
                    _ => None,
                },
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
        // When nested inside a TSK container, skip proxy/network setup since the
        // outer container already provides network isolation.
        let resolved = resolve_config_from_task(task, &self.ctx, &self.event_sender);
        let proxy_config = resolved.proxy_config();

        let mut proxy_ip: Option<String> = None;
        let (network_name, proxy_container_name) = if task.network_isolation && !self.is_nested() {
            let proxy_build_log_path = self
                .ctx
                .tsk_env()
                .task_dir(&task.id)
                .join("output")
                .join("proxy-build.log");
            let pcn = match self
                .proxy_manager
                .ensure_proxy(&proxy_config, Some(&proxy_build_log_path))
                .await
            {
                Ok(name) => name,
                Err(e) => {
                    return Err(format!(
                        "Failed to ensure proxy is running and healthy: {e}. \
                        The task should be retried later when the proxy is available. \
                        Check the status in Docker."
                    ));
                }
            };

            let name = self
                .proxy_manager
                .create_agent_network(&task.id)
                .await
                .map_err(|e| format!("Failed to create agent network: {e}"))?;

            if let Err(e) = self
                .proxy_manager
                .connect_proxy_to_network(&pcn, &name)
                .await
            {
                self.proxy_manager.cleanup_agent_network(&pcn, &name).await;
                return Err(format!("Failed to connect proxy to agent network: {e}"));
            }

            // Resolve the proxy IP on the agent network (not the external network)
            // so the extra_hosts entry points to an IP routable from the container.
            proxy_ip = match self.proxy_manager.resolve_proxy_ip(&pcn, &name).await {
                Ok(ip) => Some(ip),
                Err(e) => {
                    self.emit(ServerEvent::WarningMessage(format!(
                        "Warning: Could not resolve proxy IP for extra_hosts: {e}"
                    )));
                    None
                }
            };

            (Some(name), Some(pcn))
        } else {
            (None, None)
        };

        // --- Execute: run the container, capturing its ID for cleanup ---
        let use_proxy = network_name.is_some();
        let (container_id, result) = self
            .run_container_inner(
                docker_image_tag,
                task,
                agent,
                network_name.as_deref(),
                if use_proxy { Some(&proxy_config) } else { None },
                proxy_ip.as_deref(),
            )
            .await;

        // --- Cleanup: always runs regardless of success/failure ---
        if let Some(ref id) = container_id {
            let _ = self.remove_container(id).await;
        }
        if let (Some(name), Some(pcn)) = (&network_name, &proxy_container_name) {
            self.proxy_manager.cleanup_agent_network(pcn, name).await;
        }
        if use_proxy && let Err(e) = self.proxy_manager.maybe_stop_proxy(&proxy_config).await {
            self.emit(ServerEvent::WarningMessage(format!(
                "Warning: Failed to check/stop idle proxy: {e}"
            )));
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
        proxy_config: Option<&crate::context::ResolvedProxyConfig>,
        proxy_container_ip: Option<&str>,
    ) -> (
        Option<String>,
        Result<(String, crate::agent::TaskResult), String>,
    ) {
        let suppress_stdout = self.event_sender.is_some();
        let config = self.create_container_config(
            docker_image_tag,
            task,
            agent,
            network_name,
            proxy_config,
            proxy_container_ip,
        );
        let container_name = self.build_container_name(task);
        let options = bollard::query_parameters::CreateContainerOptionsBuilder::default()
            .name(&container_name)
            .build();

        let container_id = match self.client.create_container(Some(options), config).await {
            Ok(id) => id,
            Err(e) => return (None, Err(e)),
        };

        // Copy agent files into container before starting
        for (tar_data, dest_path) in agent.files_to_copy() {
            if let Err(e) = self
                .client
                .upload_to_container(&container_id, &dest_path, tar_data)
                .await
            {
                return (Some(container_id), Err(e));
            }
        }

        if let Err(e) = self.client.start_container(&container_id).await {
            return (Some(container_id), Err(e));
        }

        let result = if task.is_interactive {
            println!("\nStarting interactive session...");

            match self.client.attach_container(&container_id).await {
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
            // Open a log file to persist processed agent output
            let log_file = {
                let output_dir = self.ctx.tsk_env().task_dir(&task.id).join("output");
                if let Err(e) = std::fs::create_dir_all(&output_dir) {
                    self.emit(ServerEvent::WarningMessage(format!(
                        "Warning: Failed to create output directory: {e}"
                    )));
                    None
                } else {
                    match std::fs::File::create(output_dir.join("agent.log")) {
                        Ok(file) => Some(file),
                        Err(e) => {
                            self.emit(ServerEvent::WarningMessage(format!(
                                "Warning: Failed to create agent log file: {e}"
                            )));
                            None
                        }
                    }
                }
            };

            let mut log_processor = agent.create_log_processor(Some(task));
            let output = self
                .stream_container_logs(
                    &container_id,
                    &mut *log_processor,
                    log_file,
                    suppress_stdout,
                )
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
        log_file: Option<std::fs::File>,
        suppress_stdout: bool,
    ) -> Result<(String, i64), String> {
        let mut log_file = log_file;
        // Start a background task to stream logs
        let client_clone = Arc::clone(&self.client);
        let container_id_clone = container_id.to_string();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);
        let log_event_sender = self.event_sender.clone();

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
                                crate::tui::events::emit_or_print(
                                    &log_event_sender,
                                    crate::tui::events::ServerEvent::WarningMessage(format!(
                                        "Error streaming logs: {e}"
                                    )),
                                );
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    crate::tui::events::emit_or_print(
                        &log_event_sender,
                        crate::tui::events::ServerEvent::WarningMessage(format!(
                            "Failed to start log streaming: {e}"
                        )),
                    );
                }
            }
        });

        // Collect all logs for return value
        let mut all_logs = String::new();

        // Buffer for accumulating partial lines from Docker chunks
        let mut line_buffer = String::new();

        // Get docker client to avoid temporary value issues
        let docker_client = Arc::clone(&self.client);

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
                    process_complete_lines(&mut line_buffer, log_processor, log_file.as_mut(), suppress_stdout, &self.event_sender);
                }
                exit_code = &mut wait_future => {
                    let exit_code = exit_code?;

                    // Give a bit of time for remaining logs
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                    // Drain any remaining logs
                    while let Ok(log_chunk) = rx.try_recv() {
                        all_logs.push_str(&log_chunk);
                        line_buffer.push_str(&log_chunk);
                        process_complete_lines(&mut line_buffer, log_processor, log_file.as_mut(), suppress_stdout, &self.event_sender);
                    }

                    // Flush remaining buffer content if non-empty
                    if !line_buffer.trim().is_empty() {
                        line_buffer.push('\n');
                        process_complete_lines(&mut line_buffer, log_processor, log_file.as_mut(), suppress_stdout, &self.event_sender);
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
fn process_complete_lines(
    line_buffer: &mut String,
    log_processor: &mut dyn LogProcessor,
    mut log_file: Option<&mut std::fs::File>,
    suppress_stdout: bool,
    event_sender: &Option<ServerEventSender>,
) {
    while let Some(newline_pos) = line_buffer.find('\n') {
        let complete_line = &line_buffer[..newline_pos];
        // Handle CRLF by trimming trailing \r
        let trimmed = complete_line.trim_end_matches('\r');

        if let Some(formatted) = log_processor.process_line(trimmed) {
            if !suppress_stdout {
                println!("{formatted}");
            }
            if let Some(ref mut file) = log_file
                && let Err(e) = writeln!(file, "{formatted}")
            {
                crate::tui::events::emit_or_print(
                    event_sender,
                    ServerEvent::WarningMessage(format!(
                        "Warning: Failed to write to agent log file: {e}"
                    )),
                );
            }
        }

        // Use drain() for efficient in-place removal
        line_buffer.drain(..=newline_pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::context::ResolvedProxyConfig;
    use crate::task::{Task, TaskStatus};
    use crate::test_utils::TrackedDockerClient;
    use std::sync::Arc;

    /// Returns the proxy container name for the default proxy config (no host_ports, no squid_conf).
    fn default_proxy_container_name() -> String {
        ResolvedProxyConfig::default().proxy_container_name()
    }

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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

        // Check proxy environment variables (uses fingerprinted proxy container name)
        let pcn = default_proxy_container_name();
        let env = task_container_config.env.as_ref().unwrap();
        assert!(env.contains(&format!("HTTP_PROXY=http://{pcn}:3128")));
        assert!(env.contains(&format!("HTTPS_PROXY=http://{pcn}:3128")));
        assert!(
            env.iter().any(|e| e.starts_with("JAVA_TOOL_OPTIONS=")),
            "JAVA_TOOL_OPTIONS should be set for proxy"
        );

        // Check TSK environment variables
        assert!(env.contains(&"TSK_CONTAINER=1".to_string()));
        assert!(env.contains(&"TSK_TASK_ID=test-task-id".to_string()));
        drop(create_calls); // Release the lock

        let start_calls = mock_client.start_container_calls.lock().unwrap();
        assert_eq!(start_calls.len(), 2); // One for proxy, one for task container
        assert_eq!(start_calls[0], pcn);
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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        // The agent network name is derived from the task ID in create_test_task()
        let agent_network = "tsk-agent-test-task-id";
        let mock_client = Arc::new(TrackedDockerClient {
            inspect_container_response: serde_json::json!({
                "State": { "Health": { "Status": "healthy" } },
                "NetworkSettings": {
                    "Networks": {
                        agent_network: { "IPAddress": "172.18.0.2" }
                    }
                }
            })
            .to_string(),
            ..Default::default()
        });
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

        let task = create_test_task(false);
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let _ = manager.run_task_container("tsk/base", &task, &agent).await;

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        assert_eq!(create_calls.len(), 2); // One for proxy, one for task container

        // Check proxy container config (proxy manager creates this)
        let pcn = default_proxy_container_name();
        let (proxy_options, _proxy_config) = &create_calls[0];
        assert_eq!(proxy_options.as_ref().unwrap().name, Some(pcn.clone()));

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
        // Agent containers should have extra_hosts with proxy hostname -> IP mapping
        let extra_hosts = host_config
            .extra_hosts
            .as_ref()
            .expect("extra_hosts should be set");
        assert_eq!(extra_hosts.len(), 1);
        assert!(extra_hosts[0].contains(&pcn));
        assert!(extra_hosts[0].contains("172.18.0.2"));
        let default_resolved = crate::context::ResolvedConfig::default();
        assert_eq!(
            host_config.memory,
            Some(default_resolved.memory_limit_bytes())
        );
        assert_eq!(
            host_config.cpu_quota,
            Some(default_resolved.cpu_quota_microseconds())
        );

        let binds = host_config.binds.as_ref().unwrap();
        assert_eq!(binds.len(), 4); // workspace, claude dir, instructions, and output
        assert!(binds[0].contains("/tmp/test-repo:/workspace/default"));
        // In test mode, .claude directory is in temp directory
        assert!(binds[1].contains(":/home/agent/.claude"));
        assert!(binds[2].contains(":/instructions:ro"));
        assert!(binds[3].contains(":/output"));

        // Check proxy environment variables (uses fingerprinted proxy container name)
        let env = config.env.as_ref().unwrap();
        assert!(env.contains(&format!("HTTP_PROXY=http://{pcn}:3128")));
        assert!(env.contains(&format!("HTTPS_PROXY=http://{pcn}:3128")));
        assert!(env.contains(&format!("NO_PROXY=localhost,127.0.0.1,{pcn}")));
        assert!(env.contains(&format!("no_proxy=localhost,127.0.0.1,{pcn}")));
        assert!(
            env.iter().any(|e| e.starts_with("JAVA_TOOL_OPTIONS=")),
            "JAVA_TOOL_OPTIONS should be set for proxy"
        );

        // Non-DIND task: security_opt should be None, cap_drop should include SETUID/SETGID
        assert!(
            host_config.security_opt.is_none(),
            "security_opt should be None when dind is disabled"
        );
        let cap_drop = host_config.cap_drop.as_ref().unwrap();
        assert!(
            cap_drop.contains(&"SETUID".to_string()),
            "SETUID should be dropped when dind is disabled"
        );
        assert!(
            cap_drop.contains(&"SETGID".to_string()),
            "SETGID should be dropped when dind is disabled"
        );
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
            (pcn.clone(), "tsk-agent-test-task-id".to_string())
        );
        drop(connect_calls);

        // Verify network cleanup was called
        let disconnect_calls = mock_client.disconnect_network_calls.lock().unwrap();
        assert_eq!(disconnect_calls.len(), 1);
        assert_eq!(
            disconnect_calls[0],
            (pcn, "tsk-agent-test-task-id".to_string())
        );
        drop(disconnect_calls);

        let remove_network_calls = mock_client.remove_network_calls.lock().unwrap();
        assert_eq!(remove_network_calls.len(), 1);
        assert_eq!(remove_network_calls[0], "tsk-agent-test-task-id");
    }

    #[tokio::test]
    async fn test_run_task_container_with_instructions_file() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        use crate::context::{BindMount, SharedConfig, TskConfig, VolumeMount};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with a bind mount for the test project
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            SharedConfig {
                volumes: vec![VolumeMount::Bind(BindMount {
                    host: "/host/cache".to_string(),
                    container: "/container/cache".to_string(),
                    readonly: false,
                })],
                ..Default::default()
            },
        );
        let tsk_config = TskConfig {
            project: project_configs,
            ..Default::default()
        };

        let ctx = AppContext::builder().with_tsk_config(tsk_config).build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        use crate::context::{NamedVolume, SharedConfig, TskConfig, VolumeMount};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with a named volume for the test project
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            SharedConfig {
                volumes: vec![VolumeMount::Named(NamedVolume {
                    name: "build-cache".to_string(),
                    container: "/container/cache".to_string(),
                    readonly: false,
                })],
                ..Default::default()
            },
        );
        let tsk_config = TskConfig {
            project: project_configs,
            ..Default::default()
        };

        let ctx = AppContext::builder().with_tsk_config(tsk_config).build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        use crate::context::{BindMount, SharedConfig, TskConfig, VolumeMount};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with a readonly bind mount
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            SharedConfig {
                volumes: vec![VolumeMount::Bind(BindMount {
                    host: "/etc/ssl/certs".to_string(),
                    container: "/etc/ssl/certs".to_string(),
                    readonly: true,
                })],
                ..Default::default()
            },
        );
        let tsk_config = TskConfig {
            project: project_configs,
            ..Default::default()
        };

        let ctx = AppContext::builder().with_tsk_config(tsk_config).build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        use crate::context::{EnvVar, SharedConfig, TskConfig};
        use std::collections::HashMap;

        let mock_client = Arc::new(TrackedDockerClient::default());

        // Create TskConfig with environment variables for the test project
        let mut project_configs = HashMap::new();
        project_configs.insert(
            "default".to_string(),
            SharedConfig {
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
                ..Default::default()
            },
        );
        let tsk_config = TskConfig {
            project: project_configs,
            ..Default::default()
        };

        let ctx = AppContext::builder().with_tsk_config(tsk_config).build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        assert!(
            !env.iter().any(|e| e.starts_with("JAVA_TOOL_OPTIONS=")),
            "JAVA_TOOL_OPTIONS should NOT be set when proxy is disabled"
        );

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

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

    #[tokio::test]
    async fn test_dind_security_relaxations() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

        let mut task = create_test_task(false);
        task.dind = true;
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let _ = manager.run_task_container("tsk/base", &task, &agent).await;

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_config = &create_calls[1].1;
        let host_config = task_config.host_config.as_ref().unwrap();

        // DIND: security_opt should be set with seccomp profile and apparmor=unconfined
        let security_opt = host_config
            .security_opt
            .as_ref()
            .expect("security_opt should be Some when dind is enabled");
        assert!(
            security_opt.iter().any(|s| s.starts_with("seccomp=")),
            "Should have seccomp profile"
        );
        assert!(
            security_opt.iter().any(|s| s == "apparmor=unconfined"),
            "Should have apparmor=unconfined"
        );

        // DIND: SETUID and SETGID should NOT be in cap_drop
        let cap_drop = host_config.cap_drop.as_ref().unwrap();
        assert!(
            !cap_drop.contains(&"SETUID".to_string()),
            "SETUID should not be dropped when dind is enabled"
        );
        assert!(
            !cap_drop.contains(&"SETGID".to_string()),
            "SETGID should not be dropped when dind is enabled"
        );

        // Other capabilities should still be dropped
        assert!(cap_drop.contains(&"NET_ADMIN".to_string()));
        assert!(cap_drop.contains(&"SYS_ADMIN".to_string()));

        // DIND: BUILDAH_ISOLATION=chroot should be set for nested Podman builds
        let env = task_config.env.as_ref().unwrap();
        assert!(
            env.contains(&"BUILDAH_ISOLATION=chroot".to_string()),
            "BUILDAH_ISOLATION=chroot should be set when dind is enabled"
        );
    }

    #[tokio::test]
    async fn test_non_dind_security_defaults() {
        let mock_client = Arc::new(TrackedDockerClient::default());
        let ctx = AppContext::builder().build();
        let manager = DockerManager::new(&ctx, mock_client.clone(), None);

        let task = create_test_task(false); // dind defaults to false
        let agent = crate::agent::ClaudeAgent::with_tsk_env(ctx.tsk_env());
        let _ = manager.run_task_container("tsk/base", &task, &agent).await;

        let create_calls = mock_client.create_container_calls.lock().unwrap();
        let task_config = &create_calls[1].1;
        let host_config = task_config.host_config.as_ref().unwrap();

        // Non-DIND: security_opt should be None
        assert!(
            host_config.security_opt.is_none(),
            "security_opt should be None when dind is disabled"
        );

        // Non-DIND: SETUID and SETGID should be in cap_drop
        let cap_drop = host_config.cap_drop.as_ref().unwrap();
        assert!(
            cap_drop.contains(&"SETUID".to_string()),
            "SETUID should be dropped when dind is disabled"
        );
        assert!(
            cap_drop.contains(&"SETGID".to_string()),
            "SETGID should be dropped when dind is disabled"
        );

        // Other capabilities should still be dropped
        assert!(cap_drop.contains(&"NET_ADMIN".to_string()));
        assert!(cap_drop.contains(&"SYS_ADMIN".to_string()));
        assert!(cap_drop.contains(&"SYS_PTRACE".to_string()));
        assert!(cap_drop.contains(&"DAC_OVERRIDE".to_string()));
        assert!(cap_drop.contains(&"AUDIT_WRITE".to_string()));

        // Non-DIND: BUILDAH_ISOLATION should not be set
        let env = task_config.env.as_ref().unwrap();
        assert!(
            !env.contains(&"BUILDAH_ISOLATION=chroot".to_string()),
            "BUILDAH_ISOLATION should not be set when dind is disabled"
        );
    }

    #[test]
    fn test_save_build_log_writes_output() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let log_path = temp_dir.path().join("output").join("docker-build.log");
        let build_output = "Step 1/5: FROM ubuntu\nStep 2/5: RUN apt-get update\n";

        super::save_build_log(&log_path, build_output, &None);

        assert!(log_path.exists());
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(content, build_output);
    }
}
