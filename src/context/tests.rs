#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::context::docker_client::DockerClient;
    use async_trait::async_trait;
    use std::sync::Arc;

    // Mock Docker client for tests
    #[derive(Clone)]
    struct MockDockerClient {
        name: String,
    }

    #[async_trait]
    impl DockerClient for MockDockerClient {
        #[cfg(test)]
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        async fn create_container(
            &self,
            _options: Option<bollard::container::CreateContainerOptions<String>>,
            _config: bollard::container::Config<String>,
        ) -> Result<String, String> {
            Ok(format!("container-{}", self.name))
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
            _options: Option<bollard::container::LogsOptions<String>>,
        ) -> Result<String, String> {
            Ok(format!("logs from {}", self.name))
        }

        async fn logs_stream(
            &self,
            _id: &str,
            _options: Option<bollard::container::LogsOptions<String>>,
        ) -> Result<
            Box<dyn futures_util::Stream<Item = Result<String, String>> + Send + Unpin>,
            String,
        > {
            panic!("Not implemented for tests")
        }

        async fn remove_container(
            &self,
            _id: &str,
            _options: Option<bollard::container::RemoveContainerOptions>,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn create_network(&self, _name: &str) -> Result<String, String> {
            Ok("test-network".to_string())
        }

        async fn network_exists(&self, _name: &str) -> Result<bool, String> {
            Ok(true)
        }
    }

    #[test]
    fn test_app_context_creation() {
        let docker_client = Arc::new(MockDockerClient {
            name: "test".to_string(),
        });
        let app_context = AppContext::builder()
            .with_docker_client(docker_client)
            .build();

        // Verify we can get the docker client back
        let client = app_context.docker_client();
        assert!(client.as_any().is::<MockDockerClient>());
    }

    #[tokio::test]
    async fn test_app_context_docker_client_usage() {
        let docker_client = Arc::new(MockDockerClient {
            name: "test-client".to_string(),
        });
        let app_context = AppContext::builder()
            .with_docker_client(docker_client.clone())
            .build();

        // Test that we can use the docker client through the context
        let client = app_context.docker_client();
        let container_id = client
            .create_container(None, bollard::container::Config::default())
            .await
            .unwrap();

        assert_eq!(container_id, "container-test-client");
    }

    #[test]
    fn test_app_context_test_constructor() {
        let docker_client = Arc::new(MockDockerClient {
            name: "test".to_string(),
        });
        let app_context = AppContext::new_with_test_docker(docker_client);

        // Verify we can get the docker client back
        let client = app_context.docker_client();
        assert!(client.as_any().is::<MockDockerClient>());
    }

    #[tokio::test]
    async fn test_app_context_with_file_system() {
        use crate::context::file_system::tests::MockFileSystem;

        let docker_client = Arc::new(MockDockerClient {
            name: "test".to_string(),
        });
        let file_system =
            Arc::new(MockFileSystem::new().with_file("/test/file.txt", "test content"));

        let app_context = AppContext::builder()
            .with_docker_client(docker_client)
            .with_file_system(file_system.clone())
            .build();

        // Verify we can use the file system
        let fs = app_context.file_system();
        let content = fs
            .read_file(std::path::Path::new("/test/file.txt"))
            .await
            .unwrap();
        assert_eq!(content, "test content");
    }
}
