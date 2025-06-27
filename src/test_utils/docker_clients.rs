use crate::context::docker_client::DockerClient;
use async_trait::async_trait;
use bollard::container::{Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions};
use bollard::image::BuildImageOptions;
use futures_util::Stream;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct NoOpDockerClient;

#[async_trait]
impl DockerClient for NoOpDockerClient {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_container(
        &self,
        _options: Option<CreateContainerOptions<String>>,
        _config: Config<String>,
    ) -> Result<String, String> {
        Ok("noop-container-id".to_string())
    }

    async fn start_container(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn wait_container(&self, _id: &str) -> Result<i64, String> {
        Ok(0)
    }

    async fn logs(
        &self,
        _id: &str,
        _options: Option<LogsOptions<String>>,
    ) -> Result<String, String> {
        Ok("".to_string())
    }

    async fn logs_stream(
        &self,
        _id: &str,
        _options: Option<LogsOptions<String>>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        use futures_util::stream;
        let stream = stream::once(async { Ok("".to_string()) });
        Ok(Box::new(Box::pin(stream)))
    }

    async fn remove_container(
        &self,
        _id: &str,
        _options: Option<RemoveContainerOptions>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn create_network(&self, _name: &str) -> Result<String, String> {
        Ok("noop-network-id".to_string())
    }

    async fn network_exists(&self, _name: &str) -> Result<bool, String> {
        Ok(true)
    }

    async fn build_image(
        &self,
        _options: BuildImageOptions<String>,
        _tar_archive: Vec<u8>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        use futures_util::stream;
        let stream = stream::once(async { Ok("Building image...".to_string()) });
        Ok(Box::new(Box::pin(stream)))
    }

    async fn image_exists(&self, _tag: &str) -> Result<bool, String> {
        Ok(true)
    }
}

#[derive(Clone)]
pub struct FixedResponseDockerClient {
    pub exit_code: i64,
    pub logs_output: String,
    pub container_id: String,
    pub should_fail_start: bool,
    pub should_fail_create: bool,
    pub network_exists: bool,
}

impl Default for FixedResponseDockerClient {
    fn default() -> Self {
        Self {
            exit_code: 0,
            logs_output: "Test output".to_string(),
            container_id: "test-container-id".to_string(),
            should_fail_start: false,
            should_fail_create: false,
            network_exists: true,
        }
    }
}

#[async_trait]
impl DockerClient for FixedResponseDockerClient {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_container(
        &self,
        _options: Option<CreateContainerOptions<String>>,
        _config: Config<String>,
    ) -> Result<String, String> {
        if self.should_fail_create {
            Err("Failed to create container".to_string())
        } else {
            Ok(self.container_id.clone())
        }
    }

    async fn start_container(&self, _id: &str) -> Result<(), String> {
        if self.should_fail_start {
            Err("Failed to start container".to_string())
        } else {
            Ok(())
        }
    }

    async fn wait_container(&self, _id: &str) -> Result<i64, String> {
        Ok(self.exit_code)
    }

    async fn logs(
        &self,
        _id: &str,
        _options: Option<LogsOptions<String>>,
    ) -> Result<String, String> {
        Ok(self.logs_output.clone())
    }

    async fn logs_stream(
        &self,
        _id: &str,
        _options: Option<LogsOptions<String>>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        use futures_util::stream::StreamExt;
        let output = self.logs_output.clone();
        let stream = futures_util::stream::once(async move { Ok(output) }).boxed();
        Ok(Box::new(stream))
    }

    async fn remove_container(
        &self,
        _id: &str,
        _options: Option<RemoveContainerOptions>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn create_network(&self, _name: &str) -> Result<String, String> {
        Ok("test-network-id".to_string())
    }

    async fn network_exists(&self, _name: &str) -> Result<bool, String> {
        Ok(self.network_exists)
    }

    async fn build_image(
        &self,
        _options: BuildImageOptions<String>,
        _tar_archive: Vec<u8>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        use futures_util::stream::StreamExt;
        let stream =
            futures_util::stream::once(async { Ok("Building image...".to_string()) }).boxed();
        Ok(Box::new(stream))
    }

    async fn image_exists(&self, _tag: &str) -> Result<bool, String> {
        Ok(true)
    }
}

#[derive(Clone)]
pub struct TrackedDockerClient {
    pub create_container_calls:
        Arc<Mutex<Vec<(Option<CreateContainerOptions<String>>, Config<String>)>>>,
    pub start_container_calls: Arc<Mutex<Vec<String>>>,
    pub wait_container_calls: Arc<Mutex<Vec<String>>>,
    pub logs_calls: Arc<Mutex<Vec<(String, Option<LogsOptions<String>>)>>>,
    pub remove_container_calls: Arc<Mutex<Vec<(String, Option<RemoveContainerOptions>)>>>,

    pub exit_code: i64,
    pub logs_output: String,
    pub network_exists: bool,
    pub create_network_error: Option<String>,
}

impl Default for TrackedDockerClient {
    fn default() -> Self {
        Self {
            create_container_calls: Arc::new(Mutex::new(Vec::new())),
            start_container_calls: Arc::new(Mutex::new(Vec::new())),
            wait_container_calls: Arc::new(Mutex::new(Vec::new())),
            logs_calls: Arc::new(Mutex::new(Vec::new())),
            remove_container_calls: Arc::new(Mutex::new(Vec::new())),
            exit_code: 0,
            logs_output: "Container logs".to_string(),
            network_exists: true,
            create_network_error: None,
        }
    }
}

#[async_trait]
impl DockerClient for TrackedDockerClient {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_container(
        &self,
        options: Option<CreateContainerOptions<String>>,
        config: Config<String>,
    ) -> Result<String, String> {
        let call_count = self.create_container_calls.lock().unwrap().len();

        // Determine the result before moving config
        let result = if let Some(opt) = &options {
            if opt.name == "tsk-proxy" {
                Ok("test-proxy-container-id".to_string())
            } else if let Some(cmd) = &config.cmd {
                if cmd.contains(&"false".to_string()) {
                    Ok("test-container-id-fail".to_string())
                } else {
                    Ok(format!("test-container-id-{call_count}"))
                }
            } else {
                Ok(format!("test-container-id-{call_count}"))
            }
        } else {
            Ok(format!("test-container-id-{call_count}"))
        };

        // Now we can move config into the vector
        self.create_container_calls
            .lock()
            .unwrap()
            .push((options, config));

        result
    }

    async fn start_container(&self, id: &str) -> Result<(), String> {
        self.start_container_calls
            .lock()
            .unwrap()
            .push(id.to_string());
        Ok(())
    }

    async fn wait_container(&self, id: &str) -> Result<i64, String> {
        self.wait_container_calls
            .lock()
            .unwrap()
            .push(id.to_string());
        // Return different results based on container ID for testing
        if id == "test-container-id-fail" {
            Ok(1) // Non-zero exit code
        } else {
            Ok(self.exit_code)
        }
    }

    async fn logs(&self, id: &str, options: Option<LogsOptions<String>>) -> Result<String, String> {
        self.logs_calls
            .lock()
            .unwrap()
            .push((id.to_string(), options));
        Ok(self.logs_output.clone())
    }

    async fn logs_stream(
        &self,
        id: &str,
        options: Option<LogsOptions<String>>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        // For tests, just return the logs as a single-item stream
        let logs = self.logs(id, options).await?;
        let stream = futures_util::stream::once(async move { Ok(logs) });
        Ok(Box::new(Box::pin(stream)))
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
        Ok(())
    }

    async fn create_network(&self, _name: &str) -> Result<String, String> {
        if let Some(ref error) = self.create_network_error {
            Err(error.clone())
        } else {
            Ok("test-network-id".to_string())
        }
    }

    async fn network_exists(&self, _name: &str) -> Result<bool, String> {
        Ok(self.network_exists)
    }

    async fn build_image(
        &self,
        _options: BuildImageOptions<String>,
        _tar_archive: Vec<u8>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        use futures_util::stream::StreamExt;
        let stream =
            futures_util::stream::once(async { Ok("Building image...".to_string()) }).boxed();
        Ok(Box::new(stream))
    }

    async fn image_exists(&self, _tag: &str) -> Result<bool, String> {
        Ok(true)
    }
}
