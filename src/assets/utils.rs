//! Utilities for working with embedded assets
//!
//! This module provides helper functions for extracting embedded assets
//! to the filesystem, particularly for Docker builds that require
//! dockerfile directories to be available on disk.

use super::AssetManager;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Extract a dockerfile and its associated files to a temporary directory
///
/// This function extracts all files for a given dockerfile to a temporary directory
/// and returns the path to the extracted directory. The caller is responsible for
/// cleaning up the directory when done.
///
/// # Arguments
///
/// * `asset_manager` - The asset manager to use for retrieving dockerfile files
/// * `dockerfile_name` - The name of the dockerfile (e.g., "tsk-base", "tsk-proxy")
///
/// # Returns
///
/// The path to the temporary directory containing the extracted dockerfile files
pub fn extract_dockerfile_to_temp(
    asset_manager: &dyn AssetManager,
    dockerfile_name: &str,
) -> Result<PathBuf> {
    // Create a temporary directory
    let temp_dir = tempfile::TempDir::new().context("Failed to create temporary directory")?;
    let temp_path = temp_dir.path().to_owned();

    // Extract the main Dockerfile
    let dockerfile_content = asset_manager.get_dockerfile(dockerfile_name)?;
    let dockerfile_path = temp_path.join("Dockerfile");
    fs::write(&dockerfile_path, dockerfile_content).context("Failed to write Dockerfile")?;

    // Try to extract additional files (like squid.conf for tsk-proxy)
    // We'll try common files and ignore errors if they don't exist
    let additional_files = ["squid.conf", "entrypoint.sh", "requirements.txt"];

    for file_name in &additional_files {
        match asset_manager.get_dockerfile_file(dockerfile_name, file_name) {
            Ok(content) => {
                let file_path = temp_path.join(file_name);
                fs::write(&file_path, content)
                    .with_context(|| format!("Failed to write {}", file_name))?;
            }
            Err(_) => {
                // File doesn't exist, which is fine
                continue;
            }
        }
    }

    // Keep the directory from being deleted
    std::mem::forget(temp_dir);

    Ok(temp_path)
}

/// Extract a dockerfile to a specific directory
///
/// This function extracts all files for a given dockerfile to a specified directory.
/// The directory must exist and be writable.
///
/// # Arguments
///
/// * `asset_manager` - The asset manager to use for retrieving dockerfile files
/// * `dockerfile_name` - The name of the dockerfile (e.g., "tsk-base", "tsk-proxy")
/// * `dest_dir` - The destination directory for the extracted files
#[allow(dead_code)]
pub fn extract_dockerfile_to_dir(
    asset_manager: &dyn AssetManager,
    dockerfile_name: &str,
    dest_dir: &Path,
) -> Result<()> {
    // Ensure the destination directory exists
    if !dest_dir.exists() {
        return Err(anyhow::anyhow!(
            "Destination directory does not exist: {}",
            dest_dir.display()
        ));
    }

    // Extract the main Dockerfile
    let dockerfile_content = asset_manager.get_dockerfile(dockerfile_name)?;
    let dockerfile_path = dest_dir.join("Dockerfile");
    fs::write(&dockerfile_path, dockerfile_content).context("Failed to write Dockerfile")?;

    // Try to extract additional files
    let additional_files = ["squid.conf", "entrypoint.sh", "requirements.txt"];

    for file_name in &additional_files {
        match asset_manager.get_dockerfile_file(dockerfile_name, file_name) {
            Ok(content) => {
                let file_path = dest_dir.join(file_name);
                fs::write(&file_path, content)
                    .with_context(|| format!("Failed to write {}", file_name))?;
            }
            Err(_) => {
                // File doesn't exist, which is fine
                continue;
            }
        }
    }

    Ok(())
}

/// List all files in a dockerfile directory
///
/// This function returns a list of all files available for a given dockerfile.
/// This is useful for debugging and for knowing what files to expect.
///
/// # Arguments
///
/// * `asset_manager` - The asset manager to use
/// * `dockerfile_name` - The name of the dockerfile
///
/// # Returns
///
/// A vector of file names (not including the directory path)
#[allow(dead_code)]
pub fn list_dockerfile_files(
    asset_manager: &dyn AssetManager,
    dockerfile_name: &str,
) -> Vec<String> {
    let mut files = vec!["Dockerfile".to_string()];

    // Check for common additional files
    let additional_files = ["squid.conf", "entrypoint.sh", "requirements.txt"];

    for file_name in &additional_files {
        if asset_manager
            .get_dockerfile_file(dockerfile_name, file_name)
            .is_ok()
        {
            files.push(file_name.to_string());
        }
    }

    files
}
