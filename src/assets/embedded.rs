//! Embedded asset implementation using rust-embed
//!
//! This module provides the default asset manager that embeds dockerfiles
//! and templates directly into the TSK binary at compile time.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rust_embed::RustEmbed;

use super::AssetManager;

#[derive(RustEmbed)]
#[folder = "templates/"]
#[prefix = "templates/"]
struct Templates;

#[derive(RustEmbed)]
#[folder = "dockerfiles/"]
#[prefix = "dockerfiles/"]
struct Dockerfiles;

/// Asset manager that serves embedded assets from the binary
pub struct EmbeddedAssetManager;

impl EmbeddedAssetManager {
    /// Create a new embedded asset manager
    pub fn new() -> Self {
        Self
    }
}

impl Default for EmbeddedAssetManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AssetManager for EmbeddedAssetManager {
    fn get_template(&self, template_type: &str) -> Result<String> {
        let filename = format!("templates/{template_type}.md");

        Templates::get(&filename)
            .ok_or_else(|| anyhow!("Template '{template_type}' not found"))
            .and_then(|file| {
                String::from_utf8(file.data.to_vec())
                    .map_err(|e| anyhow!("Failed to decode template '{template_type}': {e}"))
            })
    }

    fn get_dockerfile(&self, dockerfile_name: &str) -> Result<Vec<u8>> {
        let path = format!("dockerfiles/{dockerfile_name}/Dockerfile");

        Dockerfiles::get(&path)
            .ok_or_else(|| anyhow!("Dockerfile '{dockerfile_name}' not found"))
            .map(|file| file.data.to_vec())
    }

    fn get_dockerfile_file(&self, dockerfile_name: &str, file_path: &str) -> Result<Vec<u8>> {
        let path = format!("dockerfiles/{dockerfile_name}/{file_path}");

        Dockerfiles::get(&path)
            .ok_or_else(|| {
                anyhow!("File '{file_path}' not found in dockerfile '{dockerfile_name}'")
            })
            .map(|file| file.data.to_vec())
    }

    fn list_templates(&self) -> Vec<String> {
        Templates::iter()
            .filter_map(|path| {
                if path.starts_with("templates/") && path.ends_with(".md") {
                    path.strip_prefix("templates/")
                        .and_then(|p| p.strip_suffix(".md"))
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    fn list_dockerfiles(&self) -> Vec<String> {
        use std::collections::HashSet;

        let mut dockerfiles = HashSet::new();

        for path in Dockerfiles::iter() {
            if path.starts_with("dockerfiles/") {
                if let Some(remaining) = path.strip_prefix("dockerfiles/") {
                    if let Some(dockerfile_name) = remaining.split('/').next() {
                        dockerfiles.insert(dockerfile_name.to_string());
                    }
                }
            }
        }

        let mut result: Vec<String> = dockerfiles.into_iter().collect();
        result.sort();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_asset_manager_creation() {
        let _manager = EmbeddedAssetManager::new();
    }

    #[test]
    fn test_get_template_success() {
        let manager = EmbeddedAssetManager::new();

        // Test getting a known template
        let result = manager.get_template("feat");
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("Feature"));
        assert!(content.contains("{{DESCRIPTION}}"));
    }

    #[test]
    fn test_get_template_not_found() {
        let manager = EmbeddedAssetManager::new();

        // Test getting a non-existent template
        let result = manager.get_template("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_list_templates() {
        let manager = EmbeddedAssetManager::new();

        let templates = manager.list_templates();
        assert!(!templates.is_empty());

        // Check that known templates are included
        assert!(templates.contains(&"feat".to_string()));
        assert!(templates.contains(&"fix".to_string()));
        assert!(templates.contains(&"doc".to_string()));
        assert!(templates.contains(&"plan".to_string()));
        assert!(templates.contains(&"refactor".to_string()));
    }

    #[test]
    fn test_get_dockerfile_success() {
        let manager = EmbeddedAssetManager::new();

        // Test getting the tsk-base Dockerfile
        let result = manager.get_dockerfile("tsk-base");
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(!content.is_empty());

        // Convert to string to check content
        let content_str = String::from_utf8_lossy(&content);
        assert!(content_str.contains("FROM"));
    }

    #[test]
    fn test_get_dockerfile_not_found() {
        let manager = EmbeddedAssetManager::new();

        // Test getting a non-existent dockerfile
        let result = manager.get_dockerfile("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_list_dockerfiles() {
        let manager = EmbeddedAssetManager::new();

        let dockerfiles = manager.list_dockerfiles();
        assert!(!dockerfiles.is_empty());

        // Check that known dockerfiles are included
        assert!(dockerfiles.contains(&"tsk-base".to_string()));
        assert!(dockerfiles.contains(&"tsk-proxy".to_string()));
    }

    #[test]
    fn test_get_dockerfile_file_success() {
        let manager = EmbeddedAssetManager::new();

        // Test getting squid.conf from tsk-proxy
        let result = manager.get_dockerfile_file("tsk-proxy", "squid.conf");
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(!content.is_empty());

        // Convert to string to check content
        let content_str = String::from_utf8_lossy(&content);
        assert!(content_str.contains("http_access"));
    }

    #[test]
    fn test_get_dockerfile_file_not_found() {
        let manager = EmbeddedAssetManager::new();

        // Test getting a non-existent file
        let result = manager.get_dockerfile_file("tsk-base", "nonexistent.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
