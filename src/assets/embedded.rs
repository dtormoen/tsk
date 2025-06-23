//! Embedded asset implementation using rust-embed
//!
//! This module provides the default asset manager that embeds dockerfiles
//! and templates directly into the TSK binary at compile time.

use anyhow::{anyhow, Result};
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
        let filename = format!("templates/{}.md", template_type);

        Templates::get(&filename)
            .ok_or_else(|| anyhow!("Template '{}' not found", template_type))
            .and_then(|file| {
                String::from_utf8(file.data.to_vec())
                    .map_err(|e| anyhow!("Failed to decode template '{}': {}", template_type, e))
            })
    }

    fn get_dockerfile(&self, dockerfile_name: &str) -> Result<Vec<u8>> {
        let path = format!("dockerfiles/{}/Dockerfile", dockerfile_name);

        Dockerfiles::get(&path)
            .ok_or_else(|| anyhow!("Dockerfile '{}' not found", dockerfile_name))
            .map(|file| file.data.to_vec())
    }

    fn get_dockerfile_file(&self, dockerfile_name: &str, file_path: &str) -> Result<Vec<u8>> {
        let path = format!("dockerfiles/{}/{}", dockerfile_name, file_path);

        Dockerfiles::get(&path)
            .ok_or_else(|| {
                anyhow!(
                    "File '{}' not found in dockerfile '{}'",
                    file_path,
                    dockerfile_name
                )
            })
            .map(|file| file.data.to_vec())
    }

    fn list_templates(&self) -> Vec<String> {
        Templates::iter()
            .filter_map(|path| {
                if path.starts_with("templates/") && path.ends_with(".md") {
                    let template_name = path
                        .strip_prefix("templates/")
                        .and_then(|p| p.strip_suffix(".md"))
                        .map(|s| s.to_string());
                    template_name
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
