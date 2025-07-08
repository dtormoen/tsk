use crate::commands::Command;
use crate::context::AppContext;
use crate::server::TskServer;
use async_trait::async_trait;
use std::error::Error;
use std::sync::Arc;

pub struct ServerRunCommand {
    pub workers: u32,
}

#[async_trait]
impl Command for ServerRunCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Starting TSK server with {} worker(s)...", self.workers);
        let server = TskServer::with_workers(Arc::new(ctx.clone()), self.workers);

        // Setup signal handlers for graceful shutdown
        let shutdown_signal = Arc::new(tokio::sync::Notify::new());
        let shutdown_signal_clone = shutdown_signal.clone();

        tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to listen for Ctrl+C");
            println!("\nReceived shutdown signal...");
            shutdown_signal_clone.notify_one();
        });

        // Run server until shutdown
        tokio::select! {
            result = server.run() => {
                match result {
                    Ok(_) => {},
                    Err(e) => {
                        let error_msg = e.to_string();
                        eprintln!("Server error: {error_msg}");
                        return Err(Box::new(std::io::Error::other(error_msg)));
                    }
                }
            }
            _ = shutdown_signal.notified() => {
                server.shutdown().await;
            }
        }

        println!("TSK server stopped");
        Ok(())
    }
}
