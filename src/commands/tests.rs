use super::*;

#[cfg(test)]
mod command_tests {
    use super::*;

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

        let result = cmd.execute().await;
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

        let result = cmd.execute().await;
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

        let result = cmd.execute().await;
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

        let result = cmd.execute().await;
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
