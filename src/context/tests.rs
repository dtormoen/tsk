#[cfg(test)]
mod tests {
    use super::super::*;
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
            .create_container(None, bollard::container::Config::default())
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

    #[tokio::test]
    async fn test_app_context_with_file_system() {
        use crate::context::file_system::tests::MockFileSystem;

        let docker_client = Arc::new(FixedResponseDockerClient::default());
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

    #[test]
    fn test_app_context_with_terminal_operations() {
        use std::sync::Mutex;

        // Create a mock terminal operations implementation
        #[derive(Default)]
        struct MockTerminalOperations {
            titles: Mutex<Vec<String>>,
            restore_called: Mutex<bool>,
        }

        impl TerminalOperations for MockTerminalOperations {
            fn set_title(&self, title: &str) {
                self.titles.lock().unwrap().push(title.to_string());
            }

            fn restore_title(&self) {
                *self.restore_called.lock().unwrap() = true;
            }
        }

        let mock_terminal = Arc::new(MockTerminalOperations::default());
        let app_context = AppContext::builder()
            .with_terminal_operations(mock_terminal.clone())
            .build();

        // Use terminal operations through context
        let terminal = app_context.terminal_operations();
        terminal.set_title("Test Title 1");
        terminal.set_title("Test Title 2");
        terminal.restore_title();

        // Verify the mock recorded the calls
        assert_eq!(
            *mock_terminal.titles.lock().unwrap(),
            vec!["Test Title 1", "Test Title 2"]
        );
        assert!(*mock_terminal.restore_called.lock().unwrap());
    }
}
