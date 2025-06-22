use super::*;
use crate::context::AppContext;
use std::sync::Arc;

#[cfg(test)]
mod command_tests {
    use super::*;
    use crate::test_utils::{NoOpDockerClient, NoOpTskClient};

    fn create_test_context() -> AppContext {
        AppContext::builder()
            .with_docker_client(Arc::new(NoOpDockerClient))
            .with_tsk_client(Arc::new(NoOpTskClient))
            .build()
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
            .contains("Either description or instructions must be provided"));
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
            .contains("Either description or instructions must be provided"));
    }

    #[tokio::test]
    async fn test_tasks_command_validation_no_options() {
        let cmd = TasksCommand {
            delete: None,
            clean: false,
            retry: None,
            edit: false,
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

    #[tokio::test]
    async fn test_debug_command_structure() {
        let cmd = DebugCommand {
            name: "test-debug".to_string(),
            agent: Some("claude_code".to_string()),
        };

        // Verify the command has the expected fields
        assert_eq!(cmd.name, "test-debug");
        assert_eq!(cmd.agent, Some("claude_code".to_string()));
    }

    #[test]
    fn test_docker_build_command_instantiation() {
        // Test that DockerBuildCommand can be instantiated
        let _cmd = DockerBuildCommand;
    }
}
