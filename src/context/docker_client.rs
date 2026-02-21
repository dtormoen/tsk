use async_trait::async_trait;
use bollard::Docker;
use bollard::models::{ContainerCreateBody, NetworkCreateRequest};
use bollard::query_parameters::{
    BuildImageOptions, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
};
use futures_util::stream::{Stream, StreamExt};
use is_terminal::IsTerminal;
use std::collections::HashMap;

use super::ContainerEngine;

#[cfg(not(windows))]
use tokio::signal::unix::{SignalKind, signal};

/// Strip `unix://` prefix from a socket path if present
fn strip_unix_prefix(path: &str) -> &str {
    path.strip_prefix("unix://").unwrap_or(path)
}

/// Detect the Podman API socket path
///
/// Checks in order:
/// 1. DOCKER_HOST environment variable
/// 2. `podman machine inspect` output (macOS)
/// 3. `podman info --format` output (Linux)
/// 4. XDG_RUNTIME_DIR/podman/podman.sock
/// 5. /run/user/<UID>/podman/podman.sock
fn detect_podman_socket() -> String {
    // 1. Check DOCKER_HOST env var
    if let Ok(host) = std::env::var("DOCKER_HOST") {
        let path = strip_unix_prefix(&host);
        if std::path::Path::new(path).exists() {
            return path.to_string();
        }
    }

    // 2. Try `podman machine inspect` (macOS)
    if let Ok(output) = std::process::Command::new("podman")
        .args(["machine", "inspect"])
        .output()
        && output.status.success()
    {
        let json_str = String::from_utf8_lossy(&output.stdout);
        if let Some(socket_path) = extract_machine_socket_path(&json_str) {
            let path = strip_unix_prefix(&socket_path);
            if std::path::Path::new(path).exists() {
                return path.to_string();
            }
        }
    }

    // 3. Try `podman info --format`
    if let Ok(output) = std::process::Command::new("podman")
        .args(["info", "--format", "{{.Host.RemoteSocket.Path}}"])
        .output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() && std::path::Path::new(&path).exists() {
            return path;
        }
    }

    // 4. XDG_RUNTIME_DIR/podman/podman.sock
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let path = format!("{runtime_dir}/podman/podman.sock");
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }

    // 5. /run/user/<UID>/podman/podman.sock
    let uid = unsafe { libc::getuid() };
    let path = format!("/run/user/{uid}/podman/podman.sock");
    if std::path::Path::new(&path).exists() {
        return path;
    }

    // 6. /tmp/podman-run-<UID>/podman/podman.sock (Podman's fallback when XDG_RUNTIME_DIR is unset)
    let path = format!("/tmp/podman-run-{uid}/podman/podman.sock");
    if std::path::Path::new(&path).exists() {
        return path;
    }

    // Default fallback: prefer paths in order of likelihood
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        format!("{runtime_dir}/podman/podman.sock")
    } else if std::path::Path::new(&format!("/run/user/{uid}")).exists() {
        format!("/run/user/{uid}/podman/podman.sock")
    } else {
        // Match Podman's own fallback when XDG_RUNTIME_DIR is unset
        format!("/tmp/podman-run-{uid}/podman/podman.sock")
    }
}

/// Extract the Podman machine socket path from `podman machine inspect` JSON output
fn extract_machine_socket_path(json_str: &str) -> Option<String> {
    // Parse as JSON array (podman machine inspect returns an array)
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let machines = parsed.as_array()?;
    let machine = machines.first()?;
    let socket_path = machine
        .get("ConnectionInfo")?
        .get("PodmanSocket")?
        .get("Path")?
        .as_str()?;
    Some(socket_path.to_string())
}

/// Configure Podman defaults so nested container builds work out of the box.
///
/// Creates `~/.config/containers/containers.conf` with `netns = "host"` and
/// `pidns = "host"` if no config file exists. This avoids `mount proc: Operation
/// not permitted` errors when building images inside a container (e.g., when TSK
/// itself runs inside Docker/Podman).
///
/// If a config file already exists, it is left untouched to respect user
/// customizations. A warning is printed if the existing config uses a restrictive
/// `netns` setting that may cause build failures.
fn configure_podman_defaults() {
    use std::path::PathBuf;

    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            PathBuf::from(home).join(".config")
        })
        .join("containers");

    let config_path = config_dir.join("containers.conf");

    if config_path.exists() {
        // Warn about restrictive netns setting, but not inside TSK containers
        // where netns = "none" is intentionally set for network isolation
        if std::env::var("TSK_CONTAINER").is_err()
            && let Ok(contents) = std::fs::read_to_string(&config_path)
            && contents.contains(r#"netns = "none""#)
        {
            eprintln!(
                "Warning: containers.conf has netns = \"none\" which may cause \
                 Podman build failures in nested containers.\n\
                 Consider changing to netns = \"host\" in: {}",
                config_path.display()
            );
        }
        return;
    }

    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        eprintln!("Warning: could not create containers config dir: {e}");
        return;
    }

    let config_content = "\
[containers]
default_sysctls = []
netns = \"host\"
pidns = \"host\"
";

    match std::fs::write(&config_path, config_content) {
        Ok(()) => eprintln!(
            "Created Podman config for nested container support: {}",
            config_path.display()
        ),
        Err(e) => eprintln!("Warning: could not write containers.conf: {e}"),
    }
}

/// Ensure the Podman API service is available at the given socket path
///
/// On macOS, returns an error with instructions to start the Podman machine.
/// On Linux, spawns `podman system service` and waits for the socket.
fn ensure_podman_service(socket_path: &str) -> Result<(), String> {
    if std::path::Path::new(socket_path).exists() {
        return Ok(());
    }

    if cfg!(target_os = "macos") {
        return Err(format!(
            "Podman socket not found at {socket_path}\n\n\
            Please start Podman machine:\n\
              podman machine start\n\n\
            Then try again."
        ));
    }

    // Linux: try to start the service
    eprintln!("Podman socket not found at {socket_path}, starting podman system service...");

    // Create parent directory for the socket if it doesn't exist
    if let Some(parent) = std::path::Path::new(socket_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Pass the socket path to podman so it creates the socket where we expect it
    let socket_uri = format!("unix://{socket_path}");

    // Stderr must be /dev/null, not a pipe. When stderr is piped and the
    // read end is never consumed (as happens after mem::forget below), the
    // 64 KB kernel pipe buffer fills up once the service emits enough log
    // output, blocking Go's logger goroutine and eventually deadlocking the
    // entire Podman process.
    //
    // We still detect startup failures via try_wait() on the process exit
    // code — the error message text is lost but the failure is not silent.
    let mut child = std::process::Command::new("podman")
        .args(["system", "service", "--timeout=0", &socket_uri])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start podman system service: {e}"))?;

    // Wait up to 10 seconds for the socket to appear
    for _ in 0..100 {
        if std::path::Path::new(socket_path).exists() {
            // Service started successfully, leak the child handle so it outlives this process
            std::mem::forget(child);
            return Ok(());
        }
        // Check if the process has already exited (indicating failure)
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!(
                    "Podman system service exited with {status}.\n\n\
                    Please ensure Podman is installed and configured correctly.\n\
                    Try running manually: podman system service --timeout=0"
                ));
            }
            Ok(None) => {} // Still running, continue waiting
            Err(e) => {
                eprintln!("Warning: could not check podman service status: {e}");
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Timed out waiting for socket, kill the child and report
    let _ = child.kill();
    Err(format!(
        "Podman socket did not appear at {socket_path} after 10 seconds.\n\n\
        Please ensure Podman is installed and configured correctly.\n\
        Try running manually: podman system service --timeout=0"
    ))
}

#[async_trait]
pub trait DockerClient: Send + Sync {
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions>,
        config: ContainerCreateBody,
    ) -> Result<String, String>;

    async fn start_container(&self, id: &str) -> Result<(), String>;

    async fn wait_container(&self, id: &str) -> Result<i64, String>;

    async fn kill_container(&self, id: &str) -> Result<(), String>;

    /// Get container logs as a single string
    ///
    /// Used by test utilities and debugging tools
    #[allow(dead_code)] // Used by test implementations
    async fn logs(&self, id: &str, options: Option<LogsOptions>) -> Result<String, String>;

    async fn logs_stream(
        &self,
        id: &str,
        options: Option<LogsOptions>,
    ) -> Result<Box<dyn futures_util::Stream<Item = Result<String, String>> + Send + Unpin>, String>;

    async fn remove_container(
        &self,
        id: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), String>;

    async fn create_network(&self, name: &str) -> Result<String, String>;

    async fn network_exists(&self, name: &str) -> Result<bool, String>;

    /// Create an internal network (no external route)
    ///
    /// Internal networks cannot reach the internet directly, making them ideal
    /// for isolating agent containers that must route through the proxy.
    ///
    /// # Arguments
    /// * `name` - The network name (e.g., "tsk-agent-abc123")
    ///
    /// # Returns
    /// The network ID on success
    async fn create_internal_network(&self, name: &str) -> Result<String, String>;

    /// Connect a running container to an additional network
    ///
    /// Used to connect the proxy container to each agent's isolated network.
    ///
    /// # Arguments
    /// * `container` - Container ID or name
    /// * `network` - Network name to connect to
    async fn connect_container_to_network(
        &self,
        container: &str,
        network: &str,
    ) -> Result<(), String>;

    /// Disconnect a container from a network
    ///
    /// Used during cleanup to disconnect proxy from agent networks before removal.
    ///
    /// # Arguments
    /// * `container` - Container ID or name
    /// * `network` - Network name to disconnect from
    async fn disconnect_container_from_network(
        &self,
        container: &str,
        network: &str,
    ) -> Result<(), String>;

    /// Remove a Docker network
    ///
    /// # Arguments
    /// * `name` - The network name to remove
    async fn remove_network(&self, name: &str) -> Result<(), String>;

    /// Build a Docker image from a tar archive containing a Dockerfile and associated files with streaming output
    ///
    /// # Arguments
    /// * `options` - Build options including image tag, build args, and cache settings
    /// * `tar_archive` - Tar archive containing Dockerfile and any additional files
    ///
    /// # Returns
    /// A stream of build output messages, or an error if the build fails to start
    async fn build_image(
        &self,
        options: BuildImageOptions,
        tar_archive: Vec<u8>,
    ) -> Result<Box<dyn futures_util::Stream<Item = Result<String, String>> + Send + Unpin>, String>;

    /// Check if a Docker image exists locally
    ///
    /// # Arguments
    /// * `tag` - The image tag to check (e.g., "tsk/rust/claude/web-api")
    ///
    /// # Returns
    /// True if the image exists, false otherwise
    async fn image_exists(&self, tag: &str) -> Result<bool, String>;

    /// Inspect a container to get its details
    ///
    /// # Arguments
    /// * `id` - Container ID or name
    ///
    /// # Returns
    /// Container inspection data as a JSON string
    async fn inspect_container(&self, id: &str) -> Result<String, String>;

    /// Attach to a container for interactive sessions
    ///
    /// Attaches to a running container's TTY for interactive input/output.
    /// This method handles stdin, stdout, and stderr streams for containers
    /// configured with TTY and attach options.
    ///
    /// # Arguments
    /// * `id` - Container ID or name to attach to
    ///
    /// # Returns
    /// * `Ok(())` - When the interactive session completes successfully
    /// * `Err(String)` - Error message if attachment fails
    ///
    /// # Note
    /// The container must be created with `tty: true` and appropriate attach options
    /// for this method to work properly.
    async fn attach_container(&self, id: &str) -> Result<(), String>;

    /// Upload a tar archive to a container
    ///
    /// Extracts the contents of the tar archive to the specified path inside
    /// the container. This is equivalent to `docker cp` and is used to copy
    /// files into containers before they start.
    ///
    /// # Arguments
    /// * `id` - Container ID or name
    /// * `dest_path` - Destination path in the container where the tar will be extracted
    /// * `tar_data` - The tar archive data to upload
    ///
    /// # Returns
    /// * `Ok(())` - When the upload completes successfully
    /// * `Err(String)` - Error message if the upload fails
    async fn upload_to_container(
        &self,
        id: &str,
        dest_path: &str,
        tar_data: Vec<u8>,
    ) -> Result<(), String>;

    /// Ping the Docker/Podman daemon to verify connectivity
    async fn ping(&self) -> Result<String, String>;
}

#[derive(Clone)]
pub struct DefaultDockerClient {
    docker: Docker,
}

impl DefaultDockerClient {
    pub fn new(engine: &ContainerEngine) -> Result<Self, String> {
        match engine {
            ContainerEngine::Docker => match Docker::connect_with_local_defaults() {
                Ok(docker) => Ok(Self { docker }),
                Err(e) => Err(format!(
                    "Failed to connect to Docker: {e}\n\n\
                    Please ensure Docker is installed and running:\n\
                      - On macOS: Open Docker Desktop application\n\
                      - On Linux: Run 'sudo systemctl start docker' or 'sudo service docker start'\n\
                      - Check Docker status with: 'docker ps'\n\n\
                    If Docker is running, check permissions:\n\
                      - On Linux: Ensure your user is in the docker group: 'sudo usermod -aG docker $USER'\n\
                        - Then log out and back in for group changes to take effect"
                )),
            },
            ContainerEngine::Podman => {
                configure_podman_defaults();
                let socket_path = detect_podman_socket();
                ensure_podman_service(&socket_path)?;
                let client_version = bollard::ClientVersion {
                    major_version: 1,
                    minor_version: 41,
                };
                match Docker::connect_with_unix(&socket_path, 120, &client_version) {
                    Ok(docker) => Ok(Self { docker }),
                    Err(e) => Err(format!(
                        "Failed to connect to Podman at {socket_path}: {e}\n\n\
                        Please ensure Podman is installed and running:\n\
                        - On macOS: Run 'podman machine start'\n\
                        - On Linux: Run 'podman system service --timeout=0 &'\n\
                        - Check Podman status with: 'podman info'"
                    )),
                }
            }
        }
    }
}

/// Detect the current terminal size
///
/// Returns None if terminal size cannot be detected (with warning logged)
#[cfg(not(windows))]
fn detect_terminal_size() -> Option<(u16, u16)> {
    match termion::terminal_size() {
        Ok((cols, rows)) => Some((cols, rows)),
        Err(e) => {
            eprintln!("Warning: Could not detect terminal size: {e}");
            eprintln!("Continuing with default terminal dimensions");
            None
        }
    }
}

/// Resize a container's TTY to the specified dimensions
///
/// Logs a warning on failure but does not propagate the error
#[cfg(not(windows))]
async fn resize_container(docker: &Docker, container_id: &str, width: u16, height: u16) {
    use bollard::query_parameters::ResizeContainerTTYOptionsBuilder;

    let options = ResizeContainerTTYOptionsBuilder::default()
        .w(width as i32)
        .h(height as i32)
        .build();

    if let Err(e) = docker.resize_container_tty(container_id, options).await {
        eprintln!("Warning: Failed to resize container TTY to {width}x{height}: {e}");
    }
}

#[async_trait]
impl DockerClient for DefaultDockerClient {
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions>,
        config: ContainerCreateBody,
    ) -> Result<String, String> {
        let response = self
            .docker
            .create_container(options, config)
            .await
            .map_err(|e| format!("Failed to create container: {e}"))?;
        Ok(response.id)
    }

    async fn start_container(&self, id: &str) -> Result<(), String> {
        self.docker
            .start_container(id, None::<bollard::query_parameters::StartContainerOptions>)
            .await
            .map_err(|e| format!("Failed to start container: {e}"))
    }

    async fn wait_container(&self, id: &str) -> Result<i64, String> {
        use futures_util::stream::StreamExt;

        let mut stream = self
            .docker
            .wait_container(id, None::<bollard::query_parameters::WaitContainerOptions>);
        if let Some(result) = stream.next().await {
            match result {
                Ok(wait_response) => Ok(wait_response.status_code),
                Err(e) => Err(format!("Failed to wait for container: {e}")),
            }
        } else {
            Err("Container wait stream ended unexpectedly".to_string())
        }
    }

    async fn kill_container(&self, id: &str) -> Result<(), String> {
        self.docker
            .kill_container(id, None::<bollard::query_parameters::KillContainerOptions>)
            .await
            .map_err(|e| format!("Failed to kill container: {e}"))
    }

    async fn logs(&self, id: &str, options: Option<LogsOptions>) -> Result<String, String> {
        let mut stream = self.docker.logs(id, options);
        let mut output = String::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(log) => output.push_str(&log.to_string()),
                Err(e) => return Err(format!("Failed to get logs: {e}")),
            }
        }

        Ok(output)
    }

    async fn logs_stream(
        &self,
        id: &str,
        options: Option<LogsOptions>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        let stream = self.docker.logs(id, options);
        let mapped_stream = stream.map(|result| match result {
            Ok(log) => Ok(log.to_string()),
            Err(e) => Err(format!("Failed to get logs: {e}")),
        });
        Ok(Box::new(Box::pin(mapped_stream)))
    }

    async fn remove_container(
        &self,
        id: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), String> {
        self.docker
            .remove_container(id, options)
            .await
            .map_err(|e| format!("Failed to remove container: {e}"))
    }

    async fn create_network(&self, name: &str) -> Result<String, String> {
        let options = NetworkCreateRequest {
            name: name.to_string(),
            ..Default::default()
        };

        let response = self
            .docker
            .create_network(options)
            .await
            .map_err(|e| format!("Failed to create network: {e}"))?;

        Ok(response.id)
    }

    async fn network_exists(&self, name: &str) -> Result<bool, String> {
        let mut filters = HashMap::new();
        filters.insert("name", vec![name]);

        let options = bollard::query_parameters::ListNetworksOptionsBuilder::default()
            .filters(&filters)
            .build();

        let networks = self
            .docker
            .list_networks(Some(options))
            .await
            .map_err(|e| format!("Failed to list networks: {e}"))?;

        Ok(!networks.is_empty())
    }

    async fn create_internal_network(&self, name: &str) -> Result<String, String> {
        let options = NetworkCreateRequest {
            name: name.to_string(),
            internal: Some(true),
            ..Default::default()
        };

        let response = self
            .docker
            .create_network(options)
            .await
            .map_err(|e| format!("Failed to create internal network: {e}"))?;

        Ok(response.id)
    }

    async fn connect_container_to_network(
        &self,
        container: &str,
        network: &str,
    ) -> Result<(), String> {
        use bollard::models::NetworkConnectRequest;

        let request = NetworkConnectRequest {
            container: container.to_string(),
            ..Default::default()
        };

        self.docker
            .connect_network(network, request)
            .await
            .map_err(|e| format!("Failed to connect container to network: {e}"))
    }

    async fn disconnect_container_from_network(
        &self,
        container: &str,
        network: &str,
    ) -> Result<(), String> {
        use bollard::models::NetworkDisconnectRequest;

        let request = NetworkDisconnectRequest {
            container: container.to_string(),
            force: Some(false),
        };

        self.docker
            .disconnect_network(network, request)
            .await
            .map_err(|e| format!("Failed to disconnect container from network: {e}"))
    }

    async fn remove_network(&self, name: &str) -> Result<(), String> {
        self.docker
            .remove_network(name)
            .await
            .map_err(|e| format!("Failed to remove network: {e}"))
    }

    async fn build_image(
        &self,
        options: BuildImageOptions,
        tar_archive: Vec<u8>,
    ) -> Result<Box<dyn futures_util::Stream<Item = Result<String, String>> + Send + Unpin>, String>
    {
        use futures_util::StreamExt;
        use tokio::sync::mpsc;

        let (tx, rx) = mpsc::unbounded_channel();
        let docker = self.docker.clone();

        // Spawn a task to handle the streaming
        tokio::spawn(async move {
            use bytes::Bytes;
            use http_body_util::{Either, Full};

            let body = Either::Left(Full::new(Bytes::from(tar_archive)));
            let mut stream = docker.build_image(options, None, Some(body));

            while let Some(build_info) = stream.next().await {
                match build_info {
                    Ok(info) => {
                        if let Some(error_detail) = info.error_detail {
                            let error_msg = error_detail
                                .message
                                .unwrap_or_else(|| "Unknown error".to_string());
                            let _ = tx.send(Err(format!("Docker build error: {error_msg}")));
                            break;
                        } else if let Some(stream_msg) = info.stream
                            && !stream_msg.is_empty()
                            && tx.send(Ok(stream_msg)).is_err()
                        {
                            break; // Receiver dropped
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("Failed to build image: {e}")));
                        break;
                    }
                }
            }
        });

        // Convert receiver to stream
        let receiver_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
        Ok(Box::new(Box::pin(receiver_stream)))
    }

    async fn image_exists(&self, tag: &str) -> Result<bool, String> {
        let mut filters = HashMap::new();
        filters.insert("reference", vec![tag]);

        let options = bollard::query_parameters::ListImagesOptionsBuilder::default()
            .filters(&filters)
            .build();

        let images = self
            .docker
            .list_images(Some(options))
            .await
            .map_err(|e| format!("Failed to list images: {e}"))?;

        Ok(!images.is_empty())
    }

    async fn inspect_container(&self, id: &str) -> Result<String, String> {
        let container = self
            .docker
            .inspect_container(
                id,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .map_err(|e| format!("Failed to inspect container: {e}"))?;

        serde_json::to_string(&container)
            .map_err(|e| format!("Failed to serialize container info: {e}"))
    }

    async fn attach_container(&self, id: &str) -> Result<(), String> {
        use bollard::query_parameters::AttachContainerOptionsBuilder;
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        // Check if the terminal is a TTY
        let is_tty = std::io::stdin().is_terminal();
        if !is_tty {
            return Err(
                "Interactive containers require a TTY. Please run in a terminal.".to_string(),
            );
        }

        // Detect terminal size and set initial container TTY size
        #[cfg(not(windows))]
        let terminal_size = detect_terminal_size();

        #[cfg(not(windows))]
        if let Some((cols, rows)) = terminal_size {
            resize_container(&self.docker, id, cols, rows).await;
        }

        // Attach to the container
        let attach_options = AttachContainerOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .stdin(true)
            .stream(true)
            .build();

        let bollard::container::AttachContainerResults {
            mut output,
            mut input,
        } = self
            .docker
            .attach_container(id, Some(attach_options))
            .await
            .map_err(|e| format!("Failed to attach to container: {e}"))?;

        // Set up raw mode for the terminal
        #[cfg(not(windows))]
        {
            use std::io::{Read, Write, stdout};
            use termion::raw::IntoRawMode;

            // Spawn a task to pipe stdin to the container
            let input_handle = tokio::spawn(async move {
                use std::io::BufReader;
                use termion::async_stdin;
                let stdin = async_stdin();
                let mut stdin = BufReader::new(stdin).bytes();
                loop {
                    if let Some(Ok(byte)) = stdin.next() {
                        if input.write_all(&[byte]).await.is_err() {
                            break;
                        }
                    } else {
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }
                }
            });

            // Create a channel for output bytes
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

            // Spawn a task to read from the container output
            let output_handle = tokio::spawn(async move {
                while let Some(result) = output.next().await {
                    match result {
                        Ok(data) => {
                            if tx.send(data.into_bytes().to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("Error reading container output: {e}");
                            break;
                        }
                    }
                }
            });

            // Spawn a task to monitor terminal resize events (SIGWINCH)
            let docker_clone = self.docker.clone();
            let container_id_clone = id.to_string();
            let resize_handle = tokio::spawn(async move {
                // Set up SIGWINCH signal handler
                let mut sigwinch = match signal(SignalKind::window_change()) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Warning: Could not set up terminal resize handler: {e}");
                        return;
                    }
                };

                // Monitor for resize signals
                while sigwinch.recv().await.is_some() {
                    if let Some((cols, rows)) = detect_terminal_size() {
                        resize_container(&docker_clone, &container_id_clone, cols, rows).await;
                    }
                }
            });

            // Process output in a blocking task to handle raw terminal mode
            let write_handle = tokio::task::spawn_blocking(move || {
                // Set stdout to raw mode
                let stdout = stdout();
                let mut stdout = stdout
                    .lock()
                    .into_raw_mode()
                    .map_err(|e| format!("Failed to set raw mode: {e}"))?;

                // Use blocking recv to write output
                while let Some(data) = rx.blocking_recv() {
                    if stdout.write_all(&data).is_err() {
                        break;
                    }
                    if stdout.flush().is_err() {
                        break;
                    }
                }

                Ok::<(), String>(())
            });

            // Wait for the output task to complete
            let _ = output_handle.await;

            // Abort the input task
            input_handle.abort();

            // Abort the resize monitor task
            resize_handle.abort();

            // Wait for the write task to complete
            write_handle
                .await
                .map_err(|e| format!("Failed to process output: {e}"))??;
        }

        #[cfg(windows)]
        {
            return Err("Interactive containers are not yet supported on Windows".to_string());
        }

        Ok(())
    }

    async fn upload_to_container(
        &self,
        id: &str,
        dest_path: &str,
        tar_data: Vec<u8>,
    ) -> Result<(), String> {
        use bollard::body_full;
        use bollard::query_parameters::UploadToContainerOptionsBuilder;
        use bytes::Bytes;

        let options = UploadToContainerOptionsBuilder::default()
            .path(dest_path)
            .build();

        let body = body_full(Bytes::from(tar_data));

        self.docker
            .upload_to_container(id, Some(options), body)
            .await
            .map_err(|e| format!("Failed to upload to container: {e}"))
    }

    async fn ping(&self) -> Result<String, String> {
        let version = self
            .docker
            .ping()
            .await
            .map_err(|e| format!("Docker ping failed: {e}"))?;
        Ok(version)
    }
}
