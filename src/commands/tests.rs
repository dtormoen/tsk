use super::*;
use crate::context::{docker_client::DockerClient, AppContext};
use std::sync::Arc;

#[cfg(test)]
mod command_tests {
    use super::*;

    // Mock Docker client for tests
    #[derive(Clone)]
    struct MockDockerClient;

    #[async_trait::async_trait]
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
            panic!("Docker operations not expected in command tests")
        }

        async fn start_container(&self, _id: &str) -> Result<(), String> {
            panic!("Docker operations not expected in command tests")
        }

        async fn wait_container(&self, _id: &str) -> Result<i64, String> {
            panic!("Docker operations not expected in command tests")
        }

        async fn logs(
            &self,
            _id: &str,
            _options: Option<bollard::container::LogsOptions<String>>,
        ) -> Result<String, String> {
            panic!("Docker operations not expected in command tests")
        }

        async fn logs_stream(
            &self,
            _id: &str,
            _options: Option<bollard::container::LogsOptions<String>>,
        ) -> Result<
            Box<dyn futures_util::Stream<Item = Result<String, String>> + Send + Unpin>,
            String,
        > {
            panic!("Docker operations not expected in command tests")
        }

        async fn remove_container(
            &self,
            _id: &str,
            _options: Option<bollard::container::RemoveContainerOptions>,
        ) -> Result<(), String> {
            panic!("Docker operations not expected in command tests")
        }

        async fn create_network(&self, _name: &str) -> Result<String, String> {
            panic!("Docker operations not expected in command tests")
        }

        async fn network_exists(&self, _name: &str) -> Result<bool, String> {
            panic!("Docker operations not expected in command tests")
        }
    }

    fn create_test_context() -> AppContext {
        AppContext::new(Arc::new(MockDockerClient))
    }

    #[tokio::test]
    async fn test_add_command_validation_no_input() {
        let cmd = AddCommand {
            name: "test".to_string(),
            r#type: "generic".to_string(),
            description: None,
            instructions: None,
            edit: false,
            agent: None,
            timeout: 30,
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Either --description or --instructions must be provided"));
    }

    #[tokio::test]
    async fn test_add_command_invalid_task_type() {
        let cmd = AddCommand {
            name: "test".to_string(),
            r#type: "nonexistent".to_string(),
            description: Some("test description".to_string()),
            instructions: None,
            edit: false,
            agent: None,
            timeout: 30,
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No template found for task type"));
    }

    #[tokio::test]
    async fn test_quick_command_validation_no_input() {
        let cmd = QuickCommand {
            name: "test".to_string(),
            r#type: "generic".to_string(),
            description: None,
            instructions: None,
            edit: false,
            agent: None,
            timeout: 30,
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Either --description or --instructions must be provided"));
    }

    #[tokio::test]
    async fn test_tasks_command_validation_no_options() {
        let cmd = TasksCommand {
            delete: None,
            clean: false,
        };

        let ctx = create_test_context();
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Please specify either --delete"));
    }

    #[test]
    fn test_command_trait_is_object_safe() {
        // This test ensures that the Command trait can be used as a trait object
        fn _assert_object_safe(_: &dyn Command) {}
    }
}
