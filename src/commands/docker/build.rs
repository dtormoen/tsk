use crate::commands::Command;
use crate::context::AppContext;
use crate::context::docker_client::DefaultDockerClient;
use crate::docker::image_manager::{BuildOptions, DockerImageManager};
use crate::repo_utils::find_repository_root;
use async_trait::async_trait;
use std::error::Error;
use std::sync::Arc;

/// Command to build TSK Docker images using the templating system
pub struct DockerBuildCommand {
    /// Whether to build without using Docker's cache
    pub no_cache: bool,
    /// Stack (defaults to "default")
    pub stack: Option<String>,
    /// Agent (defaults to "claude")
    pub agent: Option<String>,
    /// Project (defaults to "default")
    pub project: Option<String>,
    /// Whether to only print the resolved Dockerfile without building
    pub dry_run: bool,
    /// Whether to only build the proxy image
    pub proxy_only: bool,
}

#[async_trait]
impl Command for DockerBuildCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let docker_client = Arc::new(
            DefaultDockerClient::new(&ctx.tsk_config().container_engine)
                .map_err(|e| -> Box<dyn Error> { e.into() })?,
        );

        // Handle proxy-only build
        if self.proxy_only {
            println!("Building tsk/proxy image...");
            use crate::docker::proxy_manager::ProxyManager;
            let proxy_manager = ProxyManager::new(
                docker_client,
                ctx.tsk_env(),
                ctx.tsk_config().container_engine.clone(),
            );
            proxy_manager.build_proxy(self.no_cache, None).await?;
            println!("Successfully built Docker image: tsk/proxy");
            return Ok(());
        }

        // Auto-detect stack if not provided
        let stack = match &self.stack {
            Some(ts) => ts.clone(),
            None => {
                use crate::repo_utils::find_repository_root;
                let repo_root = find_repository_root(std::path::Path::new("."))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));

                match crate::repository::detect_stack(&repo_root).await {
                    Ok(detected) => detected,
                    Err(e) => {
                        eprintln!("Warning: Failed to detect stack: {e}. Using default.");
                        "default".to_string()
                    }
                }
            }
        };

        let agent = self.agent.as_deref().unwrap_or("claude");

        // Auto-detect project if not provided
        let project = match &self.project {
            Some(p) => Some(p.clone()),
            None => {
                use crate::repo_utils::find_repository_root;
                let repo_root = find_repository_root(std::path::Path::new("."))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));

                match crate::repository::detect_project_name(&repo_root).await {
                    Ok(detected) => Some(detected),
                    Err(e) => {
                        eprintln!("Warning: Failed to detect project name: {e}. Using default.");
                        Some("default".to_string())
                    }
                }
            }
        };

        // Get project root for Docker operations
        let project_root = find_repository_root(std::path::Path::new(".")).ok();

        // Resolve config for inline layer overrides
        let project_name = project.as_deref().unwrap_or("default");
        let project_config = project_root
            .as_deref()
            .and_then(crate::context::tsk_config::load_project_config);
        let resolved_config = ctx.tsk_config().resolve_config(
            project_name,
            project_config.as_ref(),
            project_root.as_deref(),
        );

        // Create image manager with AppContext
        let image_manager = DockerImageManager::new(ctx, docker_client.clone(), None);

        // Build the main image (with dry_run flag)
        let image_tag = image_manager
            .build_image(
                &stack,
                agent,
                project.as_deref(),
                &BuildOptions {
                    no_cache: self.no_cache,
                    dry_run: self.dry_run,
                    build_root: project_root.as_deref(),
                    build_log_path: None,
                },
                Some(&resolved_config),
            )
            .await?;

        if !self.dry_run {
            println!("Successfully built Docker image: {}", image_tag);

            // Always build proxy image as it's still needed
            println!("\nBuilding tsk/proxy image...");
            use crate::docker::proxy_manager::ProxyManager;
            let proxy_manager = ProxyManager::new(
                docker_client,
                ctx.tsk_env(),
                ctx.tsk_config().container_engine.clone(),
            );
            proxy_manager.build_proxy(self.no_cache, None).await?;
            println!("Successfully built Docker image: tsk/proxy");

            println!("\nAll Docker images built successfully!");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_build_command_creation() {
        // Test that DockerBuildCommand can be instantiated
        let _command = DockerBuildCommand {
            no_cache: false,
            stack: None,
            agent: None,
            project: None,
            dry_run: false,
            proxy_only: false,
        };
    }

    #[test]
    fn test_docker_build_command_with_options() {
        // Test that DockerBuildCommand can be instantiated with all options
        let _command = DockerBuildCommand {
            no_cache: true,
            stack: Some("rust".to_string()),
            agent: Some("claude".to_string()),
            project: Some("web-api".to_string()),
            dry_run: false,
            proxy_only: false,
        };
    }

    #[test]
    fn test_docker_build_command_dry_run() {
        // Test that DockerBuildCommand can be instantiated with dry_run
        let _command = DockerBuildCommand {
            no_cache: false,
            stack: Some("python".to_string()),
            agent: Some("claude".to_string()),
            project: Some("test-project".to_string()),
            dry_run: true,
            proxy_only: false,
        };
    }

    #[test]
    fn test_docker_build_command_proxy_only() {
        // Test that DockerBuildCommand can be instantiated with proxy_only
        let _command = DockerBuildCommand {
            no_cache: false,
            stack: None,
            agent: None,
            project: None,
            dry_run: false,
            proxy_only: true,
        };
    }
}
