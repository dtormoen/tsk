use crate::context::AppContext;
use async_trait::async_trait;
use std::error::Error;

pub mod add;
pub mod debug;
pub mod docker_build;
pub mod list;
pub mod quick;
pub mod run;
pub mod stop_proxy;
pub mod stop_server;
pub mod tasks;
pub mod templates;

pub use add::AddCommand;
pub use debug::DebugCommand;
pub use docker_build::DockerBuildCommand;
pub use list::ListCommand;
pub use quick::QuickCommand;
pub use run::RunCommand;
pub use stop_proxy::StopProxyCommand;
pub use stop_server::StopServerCommand;
pub use tasks::TasksCommand;
pub use templates::TemplatesCommand;

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
        let _cmd = DockerBuildCommand {
            no_cache: false,
            tech_stack: None,
            agent: None,
            project: None,
            dry_run: false,
        };
    }
}
