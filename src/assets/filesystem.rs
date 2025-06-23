use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;

use crate::assets::AssetManager;

/// A filesystem-based implementation of AssetManager that reads assets from a directory
pub struct FileSystemAssetManager {
    base_path: PathBuf,
}

impl FileSystemAssetManager {
    /// Creates a new FileSystemAssetManager with the given base path
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
}

#[async_trait]
impl AssetManager for FileSystemAssetManager {
    fn get_template(&self, template_type: &str) -> Result<String> {
        let template_path = self.base_path.join(format!("{}.md", template_type));

        if !template_path.exists() {
            return Err(anyhow::anyhow!("Template '{}' not found", template_type));
        }

        std::fs::read_to_string(&template_path)
            .with_context(|| format!("Failed to read template file: {}", template_path.display()))
    }

    fn get_dockerfile(&self, _dockerfile_name: &str) -> Result<Vec<u8>> {
        Err(anyhow::anyhow!(
            "Dockerfile support not implemented for FileSystemAssetManager"
        ))
    }

    fn get_dockerfile_file(&self, _dockerfile_name: &str, _file_path: &str) -> Result<Vec<u8>> {
        Err(anyhow::anyhow!(
            "Dockerfile support not implemented for FileSystemAssetManager"
        ))
    }

    fn list_templates(&self) -> Vec<String> {
        let mut templates = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&self.base_path) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        if let Some(file_name) = entry.file_name().to_str() {
                            if file_name.ends_with(".md") {
                                let template_name = file_name.trim_end_matches(".md");
                                templates.push(template_name.to_string());
                            }
                        }
                    }
                }
            }
        }

        templates.sort();
        templates
    }

    fn list_dockerfiles(&self) -> Vec<String> {
        // Not supported for filesystem-based asset manager
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_get_template_success() {
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join("templates");
        fs::create_dir(&templates_dir).unwrap();

        let template_content = "# Feature Template\n\n{{DESCRIPTION}}";
        fs::write(templates_dir.join("feature.md"), template_content).unwrap();

        let manager = FileSystemAssetManager::new(templates_dir);
        let result = manager.get_template("feature").unwrap();

        assert_eq!(result, template_content);
    }

    #[test]
    fn test_get_template_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join("templates");
        fs::create_dir(&templates_dir).unwrap();

        let manager = FileSystemAssetManager::new(templates_dir);
        let result = manager.get_template("nonexistent");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_list_templates() {
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join("templates");
        fs::create_dir(&templates_dir).unwrap();

        fs::write(templates_dir.join("feature.md"), "content").unwrap();
        fs::write(templates_dir.join("fix.md"), "content").unwrap();
        fs::write(templates_dir.join("doc.md"), "content").unwrap();
        fs::write(templates_dir.join("not-a-template.txt"), "content").unwrap();

        let manager = FileSystemAssetManager::new(templates_dir);
        let templates = manager.list_templates();

        assert_eq!(templates, vec!["doc", "feature", "fix"]);
    }

    #[test]
    fn test_list_templates_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join("templates");
        fs::create_dir(&templates_dir).unwrap();

        let manager = FileSystemAssetManager::new(templates_dir);
        let templates = manager.list_templates();

        assert!(templates.is_empty());
    }

    #[test]
    fn test_list_templates_nonexistent_directory() {
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join("nonexistent");

        let manager = FileSystemAssetManager::new(templates_dir);
        let templates = manager.list_templates();

        assert!(templates.is_empty());
    }

    #[test]
    fn test_dockerfile_methods_return_error() {
        let temp_dir = TempDir::new().unwrap();
        let manager = FileSystemAssetManager::new(temp_dir.path().to_path_buf());

        assert!(manager.get_dockerfile("test").is_err());
        assert!(manager.get_dockerfile_file("test", "file").is_err());
        assert!(manager.list_dockerfiles().is_empty());
    }
}
