//! Utilities for working with embedded assets
//!
//! This module provides helper functions for extracting embedded assets
//! to the filesystem, particularly for Docker builds that require
//! dockerfile directories to be available on disk.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Extract a dockerfile and its associated files to a temporary directory
///
/// This function extracts all files for a given dockerfile to a temporary directory
/// and returns the path to the extracted directory. The caller is responsible for
/// cleaning up the directory when done.
///
/// # Arguments
///
/// * `dockerfile_name` - The name of the dockerfile (e.g., "base/default", "tsk-proxy")
///
/// # Returns
///
/// The path to the temporary directory containing the extracted dockerfile files
pub fn extract_dockerfile_to_temp(dockerfile_name: &str) -> Result<PathBuf> {
    // Create a temporary directory
    let temp_dir = tempfile::TempDir::new().context("Failed to create temporary directory")?;
    let temp_path = temp_dir.path().to_owned();

    // Extract the main Dockerfile
    let dockerfile_content = super::embedded::get_dockerfile(dockerfile_name)?;
    let dockerfile_path = temp_path.join("Dockerfile");
    fs::write(&dockerfile_path, dockerfile_content).context("Failed to write Dockerfile")?;

    // Try to extract additional files (like squid.conf for tsk-proxy)
    // We'll try common files and ignore errors if they don't exist
    let additional_files = ["squid.conf", "entrypoint.sh", "requirements.txt"];

    for file_name in &additional_files {
        match super::embedded::get_dockerfile_file(dockerfile_name, file_name) {
            Ok(content) => {
                let file_path = temp_path.join(file_name);
                fs::write(&file_path, content)
                    .with_context(|| format!("Failed to write {file_name}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_extraction() {
        let result = extract_dockerfile_to_temp("base/default");
        assert!(result.is_ok());

        let temp_dir = result.unwrap();
        assert!(temp_dir.exists());
        assert!(temp_dir.join("Dockerfile").exists());

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_asset_extraction_with_additional_files() {
        let result = extract_dockerfile_to_temp("tsk-proxy");
        assert!(result.is_ok());

        let temp_dir = result.unwrap();
        assert!(temp_dir.exists());
        assert!(temp_dir.join("Dockerfile").exists());
        assert!(temp_dir.join("squid.conf").exists());

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
