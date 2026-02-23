use crate::commands::Command;
use crate::context::AppContext;
use crate::context::docker_client::{DefaultDockerClient, DockerClient};
use crate::server::TskServer;
use async_trait::async_trait;
use std::error::Error;
use std::sync::Arc;
use tokio::signal::unix::{SignalKind, signal};

pub struct ServerStartCommand {
    pub workers: u32,
    pub quit: bool,
    pub sound: bool,
}

#[async_trait]
impl Command for ServerStartCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Starting TSK server with {} worker(s)...", self.workers);
        ctx.notification_client().set_sound_enabled(self.sound);
        let docker_client: Arc<dyn DockerClient> = Arc::new(
            DefaultDockerClient::new(&ctx.tsk_config().container_engine)
                .map_err(|e| -> Box<dyn Error> { e.into() })?,
        );

        // Validate Docker connectivity before committing to start
        docker_client.ping().await.map_err(|e| -> Box<dyn Error> {
            format!("Cannot start server: Docker/Podman daemon is not reachable: {e}").into()
        })?;

        let server = TskServer::with_workers(
            Arc::new(ctx.clone()),
            docker_client,
            self.workers,
            self.quit,
            None,
        );

        // Setup signal handlers for graceful shutdown (SIGINT and SIGTERM)
        let shutdown_signal = Arc::new(tokio::sync::Notify::new());
        let shutdown_signal_clone = shutdown_signal.clone();

        tokio::spawn(async move {
            let mut sigterm =
                signal(SignalKind::terminate()).expect("Failed to listen for SIGTERM");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = sigterm.recv() => {},
            }
            shutdown_signal_clone.notify_one();
        });

        // Run server until shutdown signal or natural quit
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
                println!("\nReceived shutdown signal, killing running containers...");
                println!("Press Ctrl-C again to force exit");

                // Install second signal handler for force exit
                tokio::spawn(async {
                    let mut sigterm =
                        signal(SignalKind::terminate()).expect("Failed to listen for SIGTERM");
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {},
                        _ = sigterm.recv() => {},
                    }
                    std::process::exit(1);
                });

                // Perform graceful shutdown
                server.graceful_shutdown().await;
            }
        }

        println!("TSK server stopped");
        Ok(())
    }
}
