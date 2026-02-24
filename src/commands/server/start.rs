use crate::commands::Command;
use crate::context::AppContext;
use crate::context::docker_client::{DefaultDockerClient, DockerClient};
use crate::server::TskServer;
use crate::tui::run::run_tui;
use async_trait::async_trait;
use std::error::Error;
use std::io::IsTerminal;
use std::sync::Arc;
use tokio::signal::unix::{SignalKind, signal};

pub struct ServerStartCommand {
    pub workers: u32,
    pub quit: bool,
    pub sound: bool,
}

/// Spawn a background task that forces process exit on the next SIGINT or SIGTERM.
/// Used after initiating graceful shutdown so that a second signal forces immediate exit.
fn spawn_force_exit_handler() {
    tokio::spawn(async {
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to listen for SIGTERM");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
        std::process::exit(1);
    });
}

#[async_trait]
impl Command for ServerStartCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let is_interactive = std::io::stdout().is_terminal();

        if !is_interactive {
            println!("Starting TSK server with {} worker(s)...", self.workers);
        }
        ctx.notification_client().set_sound_enabled(self.sound);
        let docker_client: Arc<dyn DockerClient> = Arc::new(
            DefaultDockerClient::new(&ctx.tsk_config().container_engine)
                .map_err(|e| -> Box<dyn Error> { e.into() })?,
        );

        // Validate Docker connectivity before committing to start
        docker_client.ping().await.map_err(|e| -> Box<dyn Error> {
            format!("Cannot start server: Docker/Podman daemon is not reachable: {e}").into()
        })?;

        // Create event channel for TUI mode
        let (event_sender, event_receiver) = if is_interactive {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let docker_client_for_tui = docker_client.clone();
        let server = TskServer::with_workers(
            Arc::new(ctx.clone()),
            docker_client,
            self.workers,
            self.quit,
            event_sender,
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

        if let Some(event_receiver) = event_receiver {
            // TUI mode: run TUI alongside server
            let tui_shutdown = shutdown_signal.clone();
            let storage = ctx.task_storage();
            let data_dir = ctx.tsk_env().data_dir().to_path_buf();
            let workers_total = self.workers as usize;

            tokio::select! {
                result = server.run() => {
                    match result {
                        Ok(_) => {},
                        Err(e) => {
                            let error_msg = e.to_string();
                            return Err(Box::new(std::io::Error::other(error_msg)));
                        }
                    }
                }
                tui_result = run_tui(event_receiver, storage, docker_client_for_tui, data_dir, workers_total, tui_shutdown) => {
                    if let Err(e) = tui_result {
                        eprintln!("TUI error: {e}");
                    }
                    // TUI quit (user pressed 'q') — perform graceful shutdown
                    server.graceful_shutdown().await;
                }
                _ = shutdown_signal.notified() => {
                    spawn_force_exit_handler();
                    server.graceful_shutdown().await;
                }
            }
        } else {
            // Non-interactive mode: existing plain text behavior
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
                    spawn_force_exit_handler();
                    server.graceful_shutdown().await;
                }
            }

            println!("TSK server stopped");
        }

        Ok(())
    }
}
