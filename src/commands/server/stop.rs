use crate::commands::Command;
use crate::context::AppContext;
use crate::server::lifecycle::ServerLifecycle;
use async_trait::async_trait;
use std::error::Error;

pub struct ServerStopCommand;

#[async_trait]
impl Command for ServerStopCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Stopping TSK server...");

        let lifecycle = ServerLifecycle::new(ctx.tsk_env());

        let pid = match lifecycle.read_pid() {
            Some(pid) if lifecycle.is_server_running() => pid,
            _ => {
                println!("Server is not running");
                return Ok(());
            }
        };

        if !lifecycle.send_sigterm(pid) {
            eprintln!("Failed to send SIGTERM to server process (PID {pid})");
            return Err(Box::new(std::io::Error::other(format!(
                "Failed to send SIGTERM to process {pid}"
            ))));
        }

        let poll_interval = tokio::time::Duration::from_millis(200);
        let timeout = tokio::time::Duration::from_secs(10);
        let start = tokio::time::Instant::now();

        while start.elapsed() < timeout {
            tokio::time::sleep(poll_interval).await;
            if !lifecycle.is_server_running() {
                println!("Server stopped successfully");
                return Ok(());
            }
        }

        eprintln!("Warning: Server may still be running after timeout");
        Ok(())
    }
}
