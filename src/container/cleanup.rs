//! Orphan container cleanup
//!
//! This module handles detection and cleanup of orphaned containers
//! from a previous container engine when switching between Docker and Podman.

use crate::container::{ContainerEngine, default_socket_path};
use crate::context::docker_client::DefaultDockerClient;
use crate::context::tsk_config::TskConfig;
use bollard::query_parameters::RemoveContainerOptions;
use std::io::{self, Write};
use std::sync::Arc;

#[allow(dead_code)] // Used in tests
const PROXY_CONTAINER_NAME: &str = "tsk-proxy";
#[allow(dead_code)] // Used in tests
const TSK_NETWORK_NAME: &str = "tsk-network";

/// Check if orphan cleanup is needed and prompt the user
///
/// Returns true if cleanup was performed or skipped, false if error
#[allow(dead_code)] // Future integration
pub async fn check_and_cleanup_orphans(
    tsk_config: &TskConfig,
    current_engine: ContainerEngine,
) -> Result<(), String> {
    let config_file = tsk_config.load_config_file().map_err(|e| e.to_string())?;

    // Check if we switched engines
    let last_used = match &config_file.engine.last_used {
        Some(last) => last.parse::<ContainerEngine>().ok(),
        None => None,
    };

    let other_engine = match current_engine {
        ContainerEngine::Docker => ContainerEngine::Podman,
        ContainerEngine::Podman => ContainerEngine::Docker,
    };

    // If last_used matches the other engine, we may have orphans
    if last_used == Some(other_engine) {
        // Check if the other engine is available
        if let Some(socket_path) = default_socket_path(other_engine) {
            // Try to connect and check for orphans
            let client = DefaultDockerClient::with_socket(&socket_path);
            let client = Arc::new(client);

            // Check if proxy container exists
            let has_orphans = check_orphan_container(&client).await;

            if has_orphans {
                prompt_cleanup(&client, other_engine).await?;
            }
        }
    }

    Ok(())
}

#[allow(dead_code)] // Used by check_and_cleanup_orphans
async fn check_orphan_container(client: &Arc<DefaultDockerClient>) -> bool {
    use crate::context::docker_client::DockerClient;

    // Try to inspect the proxy container
    match client.inspect_container(PROXY_CONTAINER_NAME).await {
        Ok(_) => true,
        Err(e) if e.contains("No such container") => false,
        Err(_) => false,
    }
}

#[allow(dead_code)] // Used by check_and_cleanup_orphans
async fn prompt_cleanup(
    client: &Arc<DefaultDockerClient>,
    other_engine: ContainerEngine,
) -> Result<(), String> {
    use crate::context::docker_client::DockerClient;

    println!(
        "\nFound orphaned {} containers from previous engine:",
        other_engine
    );
    println!("  - {} (container)", PROXY_CONTAINER_NAME);
    println!("  - {} (network)", TSK_NETWORK_NAME);
    print!("Remove them? [Y/n]: ");
    io::stdout().flush().map_err(|e| e.to_string())?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| e.to_string())?;

    let input = input.trim().to_lowercase();
    if input.is_empty() || input == "y" || input == "yes" {
        // Remove container
        match client
            .remove_container(
                PROXY_CONTAINER_NAME,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(_) => println!("Removed {} container", PROXY_CONTAINER_NAME),
            Err(e) if e.contains("No such container") => (),
            Err(e) => eprintln!("Warning: Failed to remove container: {}", e),
        }

        // Note: Network removal would require additional DockerClient method
        // For now, just inform the user
        println!(
            "Note: Run '{} network rm {}' to remove the orphaned network",
            other_engine, TSK_NETWORK_NAME
        );
    } else {
        println!("Skipping cleanup");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_names() {
        assert_eq!(PROXY_CONTAINER_NAME, "tsk-proxy");
        assert_eq!(TSK_NETWORK_NAME, "tsk-network");
    }
}
