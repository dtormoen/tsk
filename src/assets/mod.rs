//! Asset management system for TSK
//!
//! This module provides an abstraction over asset storage and retrieval,
//! allowing TSK to embed assets in the binary while maintaining flexibility
//! for future extensions like user-specific or repository-specific assets.

use anyhow::Result;
use async_trait::async_trait;

pub mod embedded;
pub mod utils;

#[cfg(test)]
mod tests;

/// Trait for managing TSK assets including templates and dockerfiles
#[async_trait]
pub trait AssetManager: Send + Sync {
    /// Get a template by type (e.g., "feature", "fix", "doc")
    fn get_template(&self, template_type: &str) -> Result<String>;

    /// Get a dockerfile as raw bytes
    fn get_dockerfile(&self, dockerfile_name: &str) -> Result<Vec<u8>>;

    /// Get a specific file from a dockerfile directory
    fn get_dockerfile_file(&self, dockerfile_name: &str, file_path: &str) -> Result<Vec<u8>>;

    /// List all available templates
    fn list_templates(&self) -> Vec<String>;

    /// List all available dockerfiles
    #[allow(dead_code)]
    fn list_dockerfiles(&self) -> Vec<String>;
}
