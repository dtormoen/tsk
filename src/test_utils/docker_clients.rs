use crate::context::docker_client::DockerClient;
use async_trait::async_trait;
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{
    BuildImageOptions, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
};
use futures_util::Stream;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct NoOpDockerClient;

#[async_trait]
impl DockerClient for NoOpDockerClient {
    async fn create_container(
        &self,
        _options: Option<CreateContainerOptions>,
        _config: ContainerCreateBody,
    ) -> Result<String, String> {
        Ok("noop-container-id".to_string())
    }

    async fn start_container(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn wait_container(&self, _id: &str) -> Result<i64, String> {
        Ok(0)
    }

    async fn kill_container(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn logs(&self, _id: &str, _options: Option<LogsOptions>) -> Result<String, String> {
        Ok("".to_string())
    }

    async fn logs_stream(
        &self,
        _id: &str,
        _options: Option<LogsOptions>,
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

    async fn create_internal_network(&self, _name: &str) -> Result<String, String> {
        Ok("noop-internal-network-id".to_string())
    }

    async fn connect_container_to_network(
        &self,
        _container: &str,
        _network: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn disconnect_container_from_network(
        &self,
        _container: &str,
        _network: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn remove_network(&self, _name: &str) -> Result<(), String> {
        Ok(())
    }

    async fn build_image(
        &self,
        _options: BuildImageOptions,
        _tar_archive: Vec<u8>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        use futures_util::stream;
        let stream = stream::once(async { Ok("Building image...".to_string()) });
        Ok(Box::new(Box::pin(stream)))
    }

    async fn image_exists(&self, _tag: &str) -> Result<bool, String> {
        Ok(true)
    }

    async fn inspect_container(&self, _id: &str) -> Result<String, String> {
        Ok(r#"{"State": {"Health": {"Status": "healthy"}}}"#.to_string())
    }

    async fn attach_container(&self, _id: &str) -> Result<(), String> {
        // No-op for tests - interactive sessions are not supported in test environment
        Ok(())
    }

    async fn upload_to_container(
        &self,
        _id: &str,
        _dest_path: &str,
        _tar_data: Vec<u8>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn ping(&self) -> Result<String, String> {
        Ok("OK".to_string())
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
    async fn create_container(
        &self,
        _options: Option<CreateContainerOptions>,
        _config: ContainerCreateBody,
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

    async fn kill_container(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn logs(&self, _id: &str, _options: Option<LogsOptions>) -> Result<String, String> {
        Ok(self.logs_output.clone())
    }

    async fn logs_stream(
        &self,
        _id: &str,
        _options: Option<LogsOptions>,
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

    async fn create_internal_network(&self, _name: &str) -> Result<String, String> {
        Ok("fixed-internal-network-id".to_string())
    }

    async fn connect_container_to_network(
        &self,
        _container: &str,
        _network: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn disconnect_container_from_network(
        &self,
        _container: &str,
        _network: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn remove_network(&self, _name: &str) -> Result<(), String> {
        Ok(())
    }

    async fn build_image(
        &self,
        _options: BuildImageOptions,
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

    async fn inspect_container(&self, _id: &str) -> Result<String, String> {
        Ok(r#"{"State": {"Health": {"Status": "healthy"}}}"#.to_string())
    }

    async fn attach_container(&self, _id: &str) -> Result<(), String> {
        // No-op for tests - interactive sessions are not supported in test environment
        Ok(())
    }

    async fn upload_to_container(
        &self,
        _id: &str,
        _dest_path: &str,
        _tar_data: Vec<u8>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn ping(&self) -> Result<String, String> {
        Ok("OK".to_string())
    }
}

type CreateContainerCall = (Option<CreateContainerOptions>, ContainerCreateBody);
type LogsCall = (String, Option<LogsOptions>);
type RemoveContainerCall = (String, Option<RemoveContainerOptions>);
type UploadToContainerCall = (String, String, Vec<u8>);

#[derive(Clone)]
pub struct TrackedDockerClient {
    pub create_container_calls: Arc<Mutex<Vec<CreateContainerCall>>>,
    pub start_container_calls: Arc<Mutex<Vec<String>>>,
    pub wait_container_calls: Arc<Mutex<Vec<String>>>,
    pub logs_calls: Arc<Mutex<Vec<LogsCall>>>,
    pub remove_container_calls: Arc<Mutex<Vec<RemoveContainerCall>>>,
    pub upload_to_container_calls: Arc<Mutex<Vec<UploadToContainerCall>>>,
    pub connect_network_calls: Arc<Mutex<Vec<(String, String)>>>,
    pub disconnect_network_calls: Arc<Mutex<Vec<(String, String)>>>,
    pub create_internal_network_calls: Arc<Mutex<Vec<String>>>,
    pub remove_network_calls: Arc<Mutex<Vec<String>>>,
    pub kill_container_calls: Arc<Mutex<Vec<String>>>,

    pub exit_code: i64,
    pub logs_output: String,
    pub network_exists: bool,
    pub create_network_error: Option<String>,
    pub create_internal_network_error: Option<String>,
    pub remove_network_error: Option<String>,
    pub create_container_error: Option<String>,
    pub start_container_error: Option<String>,
    pub image_exists_returns: bool,
    pub inspect_container_response: String,
}

impl Default for TrackedDockerClient {
    fn default() -> Self {
        Self {
            create_container_calls: Arc::new(Mutex::new(Vec::new())),
            start_container_calls: Arc::new(Mutex::new(Vec::new())),
            wait_container_calls: Arc::new(Mutex::new(Vec::new())),
            logs_calls: Arc::new(Mutex::new(Vec::new())),
            remove_container_calls: Arc::new(Mutex::new(Vec::new())),
            upload_to_container_calls: Arc::new(Mutex::new(Vec::new())),
            connect_network_calls: Arc::new(Mutex::new(Vec::new())),
            disconnect_network_calls: Arc::new(Mutex::new(Vec::new())),
            create_internal_network_calls: Arc::new(Mutex::new(Vec::new())),
            remove_network_calls: Arc::new(Mutex::new(Vec::new())),
            kill_container_calls: Arc::new(Mutex::new(Vec::new())),
            exit_code: 0,
            logs_output: "Container logs".to_string(),
            network_exists: true,
            create_network_error: None,
            create_internal_network_error: None,
            remove_network_error: None,
            create_container_error: None,
            start_container_error: None,
            image_exists_returns: true,
            inspect_container_response: r#"{"State": {"Health": {"Status": "healthy"}}}"#
                .to_string(),
        }
    }
}

#[async_trait]
impl DockerClient for TrackedDockerClient {
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions>,
        config: ContainerCreateBody,
    ) -> Result<String, String> {
        let call_count = self.create_container_calls.lock().unwrap().len();
        let is_proxy = options
            .as_ref()
            .is_some_and(|opt| opt.name == Some("tsk-proxy".to_string()));

        // Determine the result before moving config
        let result = if is_proxy {
            Ok("test-proxy-container-id".to_string())
        } else if let Some(ref error) = self.create_container_error {
            Err(error.clone())
        } else if config
            .cmd
            .as_ref()
            .is_some_and(|cmd| cmd.contains(&"false".to_string()))
        {
            Ok("test-container-id-fail".to_string())
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
        // Only fail for non-proxy containers (proxy uses the name "tsk-proxy")
        if id != "tsk-proxy"
            && let Some(ref error) = self.start_container_error
        {
            return Err(error.clone());
        }
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

    async fn kill_container(&self, id: &str) -> Result<(), String> {
        self.kill_container_calls
            .lock()
            .unwrap()
            .push(id.to_string());
        Ok(())
    }

    async fn logs(&self, id: &str, options: Option<LogsOptions>) -> Result<String, String> {
        self.logs_calls
            .lock()
            .unwrap()
            .push((id.to_string(), options));
        Ok(self.logs_output.clone())
    }

    async fn logs_stream(
        &self,
        id: &str,
        options: Option<LogsOptions>,
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
            .push((id.to_string(), options.clone()));

        // Simulate "No such container" error for testing
        if id == "non-existent-container" {
            Err("No such container".to_string())
        } else {
            Ok(())
        }
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

    async fn create_internal_network(&self, name: &str) -> Result<String, String> {
        self.create_internal_network_calls
            .lock()
            .unwrap()
            .push(name.to_string());

        if let Some(ref error) = self.create_internal_network_error {
            Err(error.clone())
        } else {
            Ok(format!("internal-network-{name}"))
        }
    }

    async fn connect_container_to_network(
        &self,
        container: &str,
        network: &str,
    ) -> Result<(), String> {
        self.connect_network_calls
            .lock()
            .unwrap()
            .push((container.to_string(), network.to_string()));
        Ok(())
    }

    async fn disconnect_container_from_network(
        &self,
        container: &str,
        network: &str,
    ) -> Result<(), String> {
        self.disconnect_network_calls
            .lock()
            .unwrap()
            .push((container.to_string(), network.to_string()));
        Ok(())
    }

    async fn remove_network(&self, name: &str) -> Result<(), String> {
        self.remove_network_calls
            .lock()
            .unwrap()
            .push(name.to_string());

        if let Some(ref error) = self.remove_network_error {
            Err(error.clone())
        } else {
            Ok(())
        }
    }

    async fn build_image(
        &self,
        _options: BuildImageOptions,
        _tar_archive: Vec<u8>,
    ) -> Result<Box<dyn Stream<Item = Result<String, String>> + Send + Unpin>, String> {
        use futures_util::stream::StreamExt;
        let stream =
            futures_util::stream::once(async { Ok("Building image...".to_string()) }).boxed();
        Ok(Box::new(stream))
    }

    async fn image_exists(&self, _tag: &str) -> Result<bool, String> {
        Ok(self.image_exists_returns)
    }

    async fn inspect_container(&self, _id: &str) -> Result<String, String> {
        Ok(self.inspect_container_response.clone())
    }

    async fn attach_container(&self, _id: &str) -> Result<(), String> {
        // No-op for tests - interactive sessions are not supported in test environment
        Ok(())
    }

    async fn upload_to_container(
        &self,
        id: &str,
        dest_path: &str,
        tar_data: Vec<u8>,
    ) -> Result<(), String> {
        self.upload_to_container_calls.lock().unwrap().push((
            id.to_string(),
            dest_path.to_string(),
            tar_data,
        ));
        Ok(())
    }

    async fn ping(&self) -> Result<String, String> {
        Ok("OK".to_string())
    }
}
