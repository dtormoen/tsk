use crate::commands::Command;
use crate::context::AppContext;
use crate::docker::image_manager::DockerImageManager;
use crate::repo_utils::find_repository_root;
use async_trait::async_trait;
use std::error::Error;

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
}

#[async_trait]
impl Command for DockerBuildCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        // Auto-detect stack if not provided
        let stack = match &self.stack {
            Some(ts) => {
                println!("Using stack: {ts}");
                ts.clone()
            }
            None => {
                use crate::repo_utils::find_repository_root;
                let repo_root = find_repository_root(std::path::Path::new("."))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));

                match crate::repository::detect_stack(&repo_root).await {
                    Ok(detected) => {
                        println!("Auto-detected stack: {detected}");
                        detected
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to detect stack: {e}. Using default.");
                        "default".to_string()
                    }
                }
            }
        };

        let agent = self.agent.as_deref().unwrap_or("claude-code");

        // Auto-detect project if not provided
        let project = match &self.project {
            Some(p) => {
                println!("Using project: {p}");
                Some(p.clone())
            }
            None => {
                use crate::repo_utils::find_repository_root;
                let repo_root = find_repository_root(std::path::Path::new("."))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));

                match crate::repository::detect_project_name(&repo_root).await {
                    Ok(detected) => {
                        println!("Auto-detected project name: {detected}");
                        Some(detected)
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to detect project name: {e}. Using default.");
                        Some("default".to_string())
                    }
                }
            }
        };

        // Get project root for Docker operations
        let project_root = find_repository_root(std::path::Path::new(".")).ok();

        // Create image manager with AppContext
        let image_manager = DockerImageManager::new(ctx, project_root.as_deref());

        // Build the main image (with dry_run flag)
        let image_tag = image_manager
            .build_image(
                &stack,
                agent,
                project.as_deref(),
                project_root.as_deref(),
                self.no_cache,
                self.dry_run,
            )
            .await?;

        if !self.dry_run {
            println!("Successfully built Docker image: {}", image_tag);

            // Always build proxy image as it's still needed
            println!("\nBuilding tsk/proxy image...");
            use crate::docker::proxy_manager::ProxyManager;
            let proxy_manager = ProxyManager::new(ctx);
            proxy_manager.build_proxy(self.no_cache).await?;
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
        };
    }
}
