//! Embedded asset implementation using rust-embed
//!
//! This module provides free functions that serve embedded dockerfiles
//! and templates directly from the TSK binary at compile time.

use anyhow::{Result, anyhow};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "templates/"]
#[prefix = "templates/"]
struct Templates;

#[derive(RustEmbed)]
#[folder = "dockerfiles/"]
#[prefix = "dockerfiles/"]
struct Dockerfiles;

/// Get a template by type (e.g., "feat", "fix", "doc")
pub fn get_template(template_type: &str) -> Result<String> {
    let filename = format!("templates/{template_type}.md");

    Templates::get(&filename)
        .ok_or_else(|| anyhow!("Template '{template_type}' not found"))
        .and_then(|file| {
            String::from_utf8(file.data.to_vec())
                .map_err(|e| anyhow!("Failed to decode template '{template_type}': {e}"))
        })
}

/// Get a dockerfile as raw bytes.
///
/// Accepts layer identifiers like `"base/default"`, `"stack/rust"`, `"agent/claude"`,
/// `"project/default"`, or `"tsk-proxy"`. Maps them to embedded paths:
/// - `"tsk-proxy"` -> `"dockerfiles/tsk-proxy/Dockerfile"`
/// - Everything else -> `"dockerfiles/{name}.dockerfile"`
pub fn get_dockerfile(dockerfile_name: &str) -> Result<Vec<u8>> {
    let path = if dockerfile_name == "tsk-proxy" {
        "dockerfiles/tsk-proxy/Dockerfile".to_string()
    } else {
        format!("dockerfiles/{dockerfile_name}.dockerfile")
    };

    Dockerfiles::get(&path)
        .ok_or_else(|| anyhow!("Dockerfile '{dockerfile_name}' not found"))
        .map(|file| file.data.to_vec())
}

/// Get a specific file from a dockerfile directory
pub fn get_dockerfile_file(dockerfile_name: &str, file_path: &str) -> Result<Vec<u8>> {
    let path = format!("dockerfiles/{dockerfile_name}/{file_path}");

    Dockerfiles::get(&path)
        .ok_or_else(|| anyhow!("File '{file_path}' not found in dockerfile '{dockerfile_name}'"))
        .map(|file| file.data.to_vec())
}

/// List all available templates
pub fn list_templates() -> Vec<String> {
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

/// List all available dockerfiles as full layer identifiers.
///
/// Returns identifiers like `["agent/claude", "base/default", "stack/rust", "tsk-proxy"]`.
/// For `.dockerfile` files, strips the `dockerfiles/` prefix and `.dockerfile` suffix.
/// For `tsk-proxy`, detects the directory containing a `Dockerfile`.
pub fn list_dockerfiles() -> Vec<String> {
    use std::collections::HashSet;

    let mut dockerfiles = HashSet::new();

    for path in Dockerfiles::iter() {
        if let Some(remaining) = path.strip_prefix("dockerfiles/") {
            if let Some(layer_id) = remaining.strip_suffix(".dockerfile") {
                // e.g. "base/default.dockerfile" -> "base/default"
                dockerfiles.insert(layer_id.to_string());
            } else if remaining.ends_with("/Dockerfile") {
                // e.g. "tsk-proxy/Dockerfile" -> "tsk-proxy"
                if let Some(dir_name) = remaining.strip_suffix("/Dockerfile") {
                    dockerfiles.insert(dir_name.to_string());
                }
            }
        }
    }

    let mut result: Vec<String> = dockerfiles.into_iter().collect();
    result.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_template_success() {
        let result = get_template("feat");
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("Feature"));
        assert!(content.contains("{{DESCRIPTION}}"));
    }

    #[test]
    fn test_get_template_not_found() {
        let result = get_template("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_list_templates() {
        let templates = list_templates();
        assert!(!templates.is_empty());

        assert!(templates.contains(&"feat".to_string()));
        assert!(templates.contains(&"fix".to_string()));
        assert!(templates.contains(&"doc".to_string()));
        assert!(templates.contains(&"plan".to_string()));
        assert!(templates.contains(&"refactor".to_string()));
    }

    #[test]
    fn test_get_dockerfile_success() {
        let result = get_dockerfile("base/default");
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(!content.is_empty());

        let content_str = String::from_utf8_lossy(&content);
        assert!(content_str.contains("FROM"));
    }

    #[test]
    fn test_get_dockerfile_not_found() {
        let result = get_dockerfile("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_list_dockerfiles() {
        let dockerfiles = list_dockerfiles();
        assert!(!dockerfiles.is_empty());

        // Should return full layer identifiers
        assert!(
            dockerfiles.contains(&"base/default".to_string()),
            "Expected 'base/default' in {:?}",
            dockerfiles
        );
        assert!(
            dockerfiles.contains(&"tsk-proxy".to_string()),
            "Expected 'tsk-proxy' in {:?}",
            dockerfiles
        );
        // Should contain stack and agent layers
        assert!(
            dockerfiles.iter().any(|d| d.starts_with("stack/")),
            "Expected at least one stack layer in {:?}",
            dockerfiles
        );
        assert!(
            dockerfiles.iter().any(|d| d.starts_with("agent/")),
            "Expected at least one agent layer in {:?}",
            dockerfiles
        );
    }

    #[test]
    fn test_get_dockerfile_file_success() {
        let result = get_dockerfile_file("tsk-proxy", "squid.conf");
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(!content.is_empty());

        let content_str = String::from_utf8_lossy(&content);
        assert!(content_str.contains("http_access"));
    }

    #[test]
    fn test_get_dockerfile_file_not_found() {
        let result = get_dockerfile_file("base/default", "nonexistent.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
