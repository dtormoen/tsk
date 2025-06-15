use crate::context::AppContext;
use async_trait::async_trait;
use std::error::Error;

pub mod add;
pub mod debug;
pub mod list;
pub mod quick;
pub mod run;
pub mod stop_proxy;
pub mod stop_server;
pub mod tasks;

#[cfg(test)]
mod tests;

pub use add::AddCommand;
pub use debug::DebugCommand;
pub use list::ListCommand;
pub use quick::QuickCommand;
pub use run::RunCommand;
pub use stop_proxy::StopProxyCommand;
pub use stop_server::StopServerCommand;
pub use tasks::TasksCommand;

#[async_trait]
pub trait Command: Send + Sync {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>>;
}
