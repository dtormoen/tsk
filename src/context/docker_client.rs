use async_trait::async_trait;
use bollard::Docker;
use bollard::models::{ContainerCreateBody, NetworkCreateRequest};
use bollard::query_parameters::{
    BuildImageOptions, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
};
use futures_util::stream::{Stream, StreamExt};
use is_terminal::IsTerminal;
use std::collections::HashMap;

#[cfg(not(windows))]
use tokio::signal::unix::{SignalKind, signal};

#[async_trait]
pub trait DockerClient: Send + Sync {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any;
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions>,
        config: ContainerCreateBody,
    ) -> Result<String, String>;

    async fn start_container(&self, id: &str) -> Result<(), String>;

    async fn wait_container(&self, id: &str) -> Result<i64, String>;

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
}

#[derive(Clone)]
#[allow(dead_code)] // Used in production code when no mock is provided
pub struct DefaultDockerClient {
    docker: Docker,
}

impl DefaultDockerClient {
    #[allow(dead_code)] // Used in production code when no mock is provided
    pub fn new() -> Self {
        match Docker::connect_with_local_defaults() {
            Ok(docker) => Self { docker },
            Err(e) => panic!(
                "Failed to connect to Docker: {e}\n\n\
                Please ensure Docker is installed and running:\n\
                  - On macOS: Open Docker Desktop application\n\
                  - On Linux: Run 'sudo systemctl start docker' or 'sudo service docker start'\n\
                  - Check Docker status with: 'docker ps'\n\n\
                If Docker is running, check permissions:\n\
                  - On Linux: Ensure your user is in the docker group: 'sudo usermod -aG docker $USER'\n\
                    - Then log out and back in for group changes to take effect"
            ),
        }
    }
}

impl Default for DefaultDockerClient {
    fn default() -> Self {
        Self::new()
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
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
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
}
