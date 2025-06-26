use super::Command;
use crate::context::AppContext;
use async_trait::async_trait;
use std::error::Error;

pub struct StopServerCommand;

#[async_trait]
impl Command for StopServerCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Stopping TSK server...");

        let client = ctx.tsk_client();

        if !client.is_server_available().await {
            println!("Server is not running");
            return Ok(());
        }

        match client.shutdown_server().await {
            Ok(_) => {
                println!("Server shutdown command sent successfully");

                // Wait a bit for the server to shut down
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                if client.is_server_available().await {
                    eprintln!("Warning: Server may still be running");
                } else {
                    println!("Server stopped successfully");
                }
            }
            Err(e) => {
                eprintln!("Failed to stop server: {e}");
                return Err(Box::new(std::io::Error::other(e.to_string())));
            }
        }

        Ok(())
    }
}
