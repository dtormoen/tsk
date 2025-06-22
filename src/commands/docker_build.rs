use super::Command;
use crate::context::AppContext;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;
use tokio::process::Command as ProcessCommand;

/// Command to build the TSK Docker images (tsk/base and tsk/proxy)
pub struct DockerBuildCommand;

#[async_trait]
impl Command for DockerBuildCommand {
    async fn execute(&self, _ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        println!("Building TSK Docker images...");

        // Get git configuration for build arguments
        let git_user_name = get_git_config("user.name").await?;
        let git_user_email = get_git_config("user.email").await?;

        // Build tsk/base image
        println!("\nBuilding tsk/base image...");
        build_base_image(&git_user_name, &git_user_email).await?;

        // Build tsk/proxy image
        println!("\nBuilding tsk/proxy image...");
        build_proxy_image().await?;

        println!("\nDocker images built successfully!");
        Ok(())
    }
}

/// Get git configuration value
async fn get_git_config(key: &str) -> Result<String, Box<dyn Error>> {
    let output = ProcessCommand::new("git")
        .args(["config", "--global", key])
        .output()
        .await?;

    if !output.status.success() {
        return Err(format!(
            "Git config '{}' not set. Please configure git with your name and email.",
            key
        )
        .into());
    }

    let value = String::from_utf8(output.stdout)?.trim().to_string();

    if value.is_empty() {
        return Err(format!(
            "Git config '{}' is empty. Please configure git with your name and email.",
            key
        )
        .into());
    }

    Ok(value)
}

/// Build the tsk/base Docker image
async fn build_base_image(git_user_name: &str, git_user_email: &str) -> Result<(), Box<dyn Error>> {
    let dockerfile_path = Path::new("dockerfiles/tsk-base");

    if !dockerfile_path.exists() {
        return Err(format!(
            "Dockerfile directory not found: {}",
            dockerfile_path.display()
        )
        .into());
    }

    let status = ProcessCommand::new("docker")
        .args([
            "build",
            "--build-arg",
            &format!("GIT_USER_NAME={}", git_user_name),
            "--build-arg",
            &format!("GIT_USER_EMAIL={}", git_user_email),
            "-t",
            "tsk/base",
            ".",
        ])
        .current_dir(dockerfile_path)
        .status()
        .await?;

    if !status.success() {
        return Err("Failed to build tsk/base image".into());
    }

    Ok(())
}

/// Build the tsk/proxy Docker image
async fn build_proxy_image() -> Result<(), Box<dyn Error>> {
    let dockerfile_path = Path::new("dockerfiles/tsk-proxy");

    if !dockerfile_path.exists() {
        return Err(format!(
            "Dockerfile directory not found: {}",
            dockerfile_path.display()
        )
        .into());
    }

    let status = ProcessCommand::new("docker")
        .args(["build", "-t", "tsk/proxy", "."])
        .current_dir(dockerfile_path)
        .status()
        .await?;

    if !status.success() {
        return Err("Failed to build tsk/proxy image".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_build_command_creation() {
        // Test that DockerBuildCommand can be instantiated
        let _command = DockerBuildCommand;
    }
}
