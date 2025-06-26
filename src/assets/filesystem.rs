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
        // Try both direct path and templates subdirectory
        let direct_path = self.base_path.join(format!("{template_type}.md"));
        let templates_path = self
            .base_path
            .join("templates")
            .join(format!("{template_type}.md"));

        let template_path = if direct_path.exists() {
            direct_path
        } else if templates_path.exists() {
            templates_path
        } else {
            return Err(anyhow::anyhow!("Template '{}' not found", template_type));
        };

        std::fs::read_to_string(&template_path)
            .with_context(|| format!("Failed to read template file: {}", template_path.display()))
    }

    fn get_dockerfile(&self, dockerfile_name: &str) -> Result<Vec<u8>> {
        let dockerfile_path = self
            .base_path
            .join("dockerfiles")
            .join(dockerfile_name)
            .join("Dockerfile");

        if !dockerfile_path.exists() {
            return Err(anyhow::anyhow!(
                "Dockerfile '{}' not found",
                dockerfile_name
            ));
        }

        std::fs::read(&dockerfile_path)
            .with_context(|| format!("Failed to read dockerfile: {}", dockerfile_path.display()))
    }

    fn get_dockerfile_file(&self, dockerfile_name: &str, file_path: &str) -> Result<Vec<u8>> {
        let file_full_path = self
            .base_path
            .join("dockerfiles")
            .join(dockerfile_name)
            .join(file_path);

        if !file_full_path.exists() {
            return Err(anyhow::anyhow!(
                "Dockerfile file '{}/{}' not found",
                dockerfile_name,
                file_path
            ));
        }

        std::fs::read(&file_full_path).with_context(|| {
            format!(
                "Failed to read dockerfile file: {}",
                file_full_path.display()
            )
        })
    }

    fn list_templates(&self) -> Vec<String> {
        let mut templates = Vec::new();

        // Check direct path
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

        // Check templates subdirectory
        let templates_dir = self.base_path.join("templates");
        if let Ok(entries) = std::fs::read_dir(&templates_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        if let Some(file_name) = entry.file_name().to_str() {
                            if file_name.ends_with(".md") {
                                let template_name = file_name.trim_end_matches(".md");
                                if !templates.contains(&template_name.to_string()) {
                                    templates.push(template_name.to_string());
                                }
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
        let mut dockerfiles = Vec::new();
        let dockerfiles_dir = self.base_path.join("dockerfiles");

        if let Ok(entries) = std::fs::read_dir(&dockerfiles_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_dir() {
                        if let Some(dir_name) = entry.file_name().to_str() {
                            // Check if this directory contains subdirectories with Dockerfiles
                            let layer_dir = dockerfiles_dir.join(dir_name);
                            if let Ok(layer_entries) = std::fs::read_dir(&layer_dir) {
                                for layer_entry in layer_entries.flatten() {
                                    if let Ok(layer_file_type) = layer_entry.file_type() {
                                        if layer_file_type.is_dir() {
                                            if let Some(layer_name) =
                                                layer_entry.file_name().to_str()
                                            {
                                                let dockerfile_path =
                                                    layer_dir.join(layer_name).join("Dockerfile");
                                                if dockerfile_path.exists() {
                                                    dockerfiles
                                                        .push(format!("{dir_name}/{layer_name}"));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        dockerfiles.sort();
        dockerfiles
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
        fs::write(templates_dir.join("feat.md"), template_content).unwrap();

        let manager = FileSystemAssetManager::new(templates_dir);
        let result = manager.get_template("feat").unwrap();

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

        fs::write(templates_dir.join("feat.md"), "content").unwrap();
        fs::write(templates_dir.join("fix.md"), "content").unwrap();
        fs::write(templates_dir.join("doc.md"), "content").unwrap();
        fs::write(templates_dir.join("not-a-template.txt"), "content").unwrap();

        let manager = FileSystemAssetManager::new(templates_dir);
        let templates = manager.list_templates();

        assert_eq!(templates, vec!["doc", "feat", "fix"]);
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
