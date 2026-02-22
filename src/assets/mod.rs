//! Asset management system for TSK
//!
//! This module provides template and dockerfile asset management. Dockerfiles
//! are served from embedded assets compiled into the binary. Templates are
//! discovered from project-level (`.tsk/templates/`), user-level
//! (`~/.config/tsk/templates/`), and embedded sources in priority order.

use anyhow::{Result, anyhow};
use std::path::Path;

pub mod embedded;
pub(crate) mod frontmatter;
pub mod utils;

use crate::context::tsk_env::TskEnv;
use embedded::EmbeddedAssetManager;

/// Find a template by name, checking project, user, and embedded sources in priority order.
pub fn find_template(name: &str, project_root: Option<&Path>, tsk_env: &TskEnv) -> Result<String> {
    let filename = format!("{name}.md");

    // Check project level first
    if let Some(root) = project_root {
        let project_path = root.join(".tsk").join("templates").join(&filename);
        if project_path.exists() {
            return std::fs::read_to_string(&project_path).map_err(|e| {
                anyhow!(
                    "Failed to read template '{}': {}",
                    project_path.display(),
                    e
                )
            });
        }
    }

    // Check user level
    let user_path = tsk_env.config_dir().join("templates").join(&filename);
    if user_path.exists() {
        return std::fs::read_to_string(&user_path)
            .map_err(|e| anyhow!("Failed to read template '{}': {}", user_path.display(), e));
    }

    // Fall back to embedded
    EmbeddedAssetManager.get_template(name)
}

/// List all available templates from project, user, and embedded sources.
pub fn list_all_templates(project_root: Option<&Path>, tsk_env: &TskEnv) -> Vec<String> {
    let mut templates = std::collections::HashSet::new();

    // Check project level
    if let Some(root) = project_root {
        scan_template_dir(&root.join(".tsk").join("templates"), &mut templates);
    }

    // Check user level
    scan_template_dir(&tsk_env.config_dir().join("templates"), &mut templates);

    // Check embedded
    for name in EmbeddedAssetManager.list_templates() {
        templates.insert(name);
    }

    let mut result: Vec<String> = templates.into_iter().collect();
    result.sort();
    result
}

fn scan_template_dir(dir: &Path, templates: &mut std::collections::HashSet<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(filename) = entry.file_name().to_str()
                && let Some(name) = filename.strip_suffix(".md")
            {
                templates.insert(name.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_find_template_embedded_fallback() {
        let ctx = AppContext::builder().build();
        let result = find_template("feat", None, &ctx.tsk_env());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("{{DESCRIPTION}}"));
    }

    #[test]
    fn test_find_template_not_found() {
        let ctx = AppContext::builder().build();
        let result = find_template("nonexistent-xyz", None, &ctx.tsk_env());
        assert!(result.is_err());
    }

    #[test]
    fn test_find_template_project_priority() {
        let ctx = AppContext::builder().build();
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join(".tsk").join("templates");
        fs::create_dir_all(&templates_dir).unwrap();
        fs::write(templates_dir.join("feat.md"), "project-level feat").unwrap();

        let result = find_template("feat", Some(temp_dir.path()), &ctx.tsk_env());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "project-level feat");
    }

    #[test]
    fn test_list_all_templates_includes_embedded() {
        let ctx = AppContext::builder().build();
        let templates = list_all_templates(None, &ctx.tsk_env());
        assert!(templates.contains(&"feat".to_string()));
        assert!(templates.contains(&"fix".to_string()));
    }

    #[test]
    fn test_list_all_templates_includes_project_and_deduplicates() {
        let ctx = AppContext::builder().build();
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join(".tsk").join("templates");
        fs::create_dir_all(&templates_dir).unwrap();
        fs::write(templates_dir.join("feat.md"), "override").unwrap();
        fs::write(templates_dir.join("custom-task.md"), "custom").unwrap();

        let templates = list_all_templates(Some(temp_dir.path()), &ctx.tsk_env());
        // Should include both embedded and project templates
        assert!(templates.contains(&"feat".to_string()));
        assert!(templates.contains(&"custom-task".to_string()));
        // Should be deduplicated (feat appears once, not twice)
        assert_eq!(
            templates.iter().filter(|t| *t == "feat").count(),
            1,
            "feat should appear exactly once"
        );
        // Should be sorted
        let mut sorted = templates.clone();
        sorted.sort();
        assert_eq!(templates, sorted);
    }
}
