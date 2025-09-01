use crate::context::AppContext;
use async_trait::async_trait;
use std::error::Error;

pub mod add;
pub mod clean;
pub mod debug;
pub mod delete;
pub mod docker;
pub mod list;
pub mod proxy;
pub mod quick;
pub mod retry;
pub mod run;
pub mod server;
pub mod template;

pub use add::AddCommand;
pub use clean::CleanCommand;
pub use debug::DebugCommand;
pub use delete::DeleteCommand;
pub use list::ListCommand;
pub use quick::QuickCommand;
pub use retry::RetryCommand;
pub use run::RunCommand;

#[async_trait]
pub trait Command: Send + Sync {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_trait_is_object_safe() {
        // This test ensures that the Command trait can be used as a trait object
        fn _assert_object_safe(_: &dyn Command) {}
    }

    #[test]
    fn test_docker_build_command_instantiation() {
        // Test that DockerBuildCommand can be instantiated
        let _cmd = docker::DockerBuildCommand {
            no_cache: false,
            stack: None,
            agent: None,
            project: None,
            dry_run: false,
        };
    }
}
