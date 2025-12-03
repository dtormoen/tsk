//! User configuration loaded from tsk.toml
//!
//! This module contains configuration types that are loaded from the user's
//! configuration file (`~/.config/tsk/tsk.toml`). These options allow users
//! to customize Docker container resources and project-specific settings.

use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::path::PathBuf;

use super::tsk_env::TskEnvError;

/// User configuration loaded from tsk.toml
///
/// This struct contains user-configurable options for TSK, including
/// Docker container resource limits and project-specific settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TskConfig {
    /// Docker container resource configuration
    pub docker: DockerOptions,
    /// Project-specific configurations keyed by project name
    #[serde(default)]
    pub project: HashMap<String, ProjectConfig>,
}

impl TskConfig {
    /// Get project-specific configuration by project name
    ///
    /// Returns `None` if no configuration exists for the given project.
    pub fn get_project_config(&self, project_name: &str) -> Option<&ProjectConfig> {
        self.project.get(project_name)
    }
}

/// Docker container resource configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DockerOptions {
    /// Container memory limit in bytes (default: 12GB)
    pub memory_limit: i64,
    /// CPU quota in microseconds per 100ms period (default: 800000 = 8 CPUs)
    pub cpu_quota: i64,
}

impl Default for DockerOptions {
    fn default() -> Self {
        Self {
            memory_limit: 12 * 1024 * 1024 * 1024, // 12GB
            cpu_quota: 800_000,                    // 8 CPUs
        }
    }
}

/// Project-specific configuration section
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    /// Default agent for this project (e.g., "claude", "codex")
    pub agent: Option<String>,
    /// Default stack for this project (e.g., "go", "rust", "python")
    pub stack: Option<String>,
    /// Volume mounts for Docker containers
    #[serde(default)]
    pub volumes: Vec<VolumeMount>,
}

/// Volume mount configuration
///
/// Supports two types of mounts:
/// 1. Bind mounts: Map a host path to a container path
/// 2. Named volumes: Use a Docker-managed named volume
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum VolumeMount {
    /// Bind mount from host filesystem
    Bind(BindMount),
    /// Docker-managed named volume
    Named(NamedVolume),
}

/// Bind mount configuration
#[derive(Debug, Clone, Deserialize)]
pub struct BindMount {
    /// Host path (supports ~ expansion)
    pub host: String,
    /// Container path
    pub container: String,
    /// Read-only flag (default: false)
    #[serde(default)]
    pub readonly: bool,
}

/// Named volume configuration (Docker-managed)
#[derive(Debug, Clone, Deserialize)]
pub struct NamedVolume {
    /// Docker volume name (will be prefixed with "tsk-" to avoid conflicts)
    pub name: String,
    /// Container path
    pub container: String,
    /// Read-only flag (default: false)
    #[serde(default)]
    pub readonly: bool,
}

impl BindMount {
    /// Expand ~ in host path to actual home directory
    pub fn expanded_host_path(&self) -> Result<PathBuf, TskEnvError> {
        if self.host.starts_with("~/") {
            let home = env::var("HOME")
                .or_else(|_| env::var("USERPROFILE"))
                .map_err(|_| TskEnvError::NoHomeDirectory)?;
            Ok(PathBuf::from(home).join(&self.host[2..]))
        } else if self.host == "~" {
            let home = env::var("HOME")
                .or_else(|_| env::var("USERPROFILE"))
                .map_err(|_| TskEnvError::NoHomeDirectory)?;
            Ok(PathBuf::from(home))
        } else {
            Ok(PathBuf::from(&self.host))
        }
    }
}

/// Load TskConfig from a configuration directory
///
/// Attempts to load and parse `tsk.toml` from the given config directory.
/// Returns default configuration if the file doesn't exist or can't be parsed.
pub fn load_config(config_dir: &Path) -> TskConfig {
    let config_file = config_dir.join("tsk.toml");
    if config_file.exists() {
        match std::fs::read_to_string(&config_file) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => return config,
                Err(e) => {
                    eprintln!("Warning: Failed to parse {}: {}", config_file.display(), e);
                }
            },
            Err(e) => {
                eprintln!("Warning: Failed to read {}: {}", config_file.display(), e);
            }
        }
    }
    TskConfig::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_docker_options_default() {
        let options = DockerOptions::default();
        assert_eq!(options.memory_limit, 12 * 1024 * 1024 * 1024); // 12GB
        assert_eq!(options.cpu_quota, 800_000); // 8 CPUs
    }

    #[test]
    fn test_tsk_config_default() {
        let config = TskConfig::default();
        assert_eq!(config.docker.memory_limit, 12 * 1024 * 1024 * 1024);
        assert_eq!(config.docker.cpu_quota, 800_000);
        assert!(config.project.is_empty());
    }

    #[test]
    fn test_tsk_config_default_has_empty_project_map() {
        let config = TskConfig::default();
        assert!(config.project.is_empty());
    }

    #[test]
    fn test_config_from_toml_file() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[docker]
memory_limit = 8589934592
cpu_quota = 400000
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        assert_eq!(config.docker.memory_limit, 8589934592); // 8GB
        assert_eq!(config.docker.cpu_quota, 400_000); // 4 CPUs
    }

    #[test]
    fn test_config_partial_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        // Only specify memory_limit, cpu_quota should use default
        let toml_content = r#"
[docker]
memory_limit = 4294967296
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        assert_eq!(config.docker.memory_limit, 4294967296); // 4GB
        assert_eq!(config.docker.cpu_quota, 800_000); // Default 8 CPUs
    }

    #[test]
    fn test_config_missing_toml_uses_defaults() {
        let temp_dir = tempfile::TempDir::new().unwrap();

        let config = load_config(temp_dir.path());

        // Should use defaults when no config file exists
        assert_eq!(config.docker.memory_limit, 12 * 1024 * 1024 * 1024);
        assert_eq!(config.docker.cpu_quota, 800_000);
    }

    #[test]
    fn test_config_invalid_toml_type_uses_defaults() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        // Invalid type: string instead of i64
        let toml_content = r#"
[docker]
memory_limit = "not-a-number"
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        // Should use defaults when TOML contains invalid types
        assert_eq!(config.docker.memory_limit, 12 * 1024 * 1024 * 1024);
        assert_eq!(config.docker.cpu_quota, 800_000);
    }

    #[test]
    fn test_project_config_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[docker]
memory_limit = 8589934592

[project.my-go-project]
agent = "claude"
stack = "go"
volumes = [
    { host = "~/.cache/go-build", container = "/home/agent/.cache/go-build" },
    { host = "/tmp/shared", container = "/shared", readonly = true }
]

[project.my-rust-project]
stack = "rust"
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        // Check project config exists and has correct values
        let go_config = config.get_project_config("my-go-project");
        assert!(go_config.is_some());
        let go_config = go_config.unwrap();
        assert_eq!(go_config.agent, Some("claude".to_string()));
        assert_eq!(go_config.stack, Some("go".to_string()));
        assert_eq!(go_config.volumes.len(), 2);

        // Check rust project has only stack set
        let rust_config = config.get_project_config("my-rust-project");
        assert!(rust_config.is_some());
        let rust_config = rust_config.unwrap();
        assert_eq!(rust_config.agent, None);
        assert_eq!(rust_config.stack, Some("rust".to_string()));
        assert!(rust_config.volumes.is_empty());

        // Non-existent project returns None
        assert!(config.get_project_config("non-existent").is_none());
    }

    #[test]
    fn test_bind_mount_path_expansion() {
        // Test with ~ expansion
        let bind_mount = BindMount {
            host: "~/.cache/go-build".to_string(),
            container: "/home/agent/.cache/go-build".to_string(),
            readonly: false,
        };

        let expanded = bind_mount.expanded_host_path().unwrap();
        assert!(!expanded.to_string_lossy().starts_with("~"));
        assert!(expanded.to_string_lossy().ends_with(".cache/go-build"));

        // Test with just ~
        let bind_mount_home = BindMount {
            host: "~".to_string(),
            container: "/home/agent".to_string(),
            readonly: false,
        };

        let expanded_home = bind_mount_home.expanded_host_path().unwrap();
        assert!(!expanded_home.to_string_lossy().starts_with("~"));
        assert!(!expanded_home.to_string_lossy().is_empty());

        // Test with absolute path (no expansion)
        let bind_mount_abs = BindMount {
            host: "/tmp/shared".to_string(),
            container: "/shared".to_string(),
            readonly: true,
        };

        let expanded_abs = bind_mount_abs.expanded_host_path().unwrap();
        assert_eq!(expanded_abs.to_string_lossy(), "/tmp/shared");
    }

    #[test]
    fn test_named_volume_config_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[project.my-go-project]
stack = "go"
volumes = [
    { name = "go-mod-cache", container = "/go/pkg/mod" },
    { name = "go-build-cache", container = "/home/agent/.cache/go-build", readonly = true }
]
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        let go_config = config.get_project_config("my-go-project").unwrap();
        assert_eq!(go_config.volumes.len(), 2);

        // Check first named volume
        match &go_config.volumes[0] {
            VolumeMount::Named(named) => {
                assert_eq!(named.name, "go-mod-cache");
                assert_eq!(named.container, "/go/pkg/mod");
                assert!(!named.readonly);
            }
            VolumeMount::Bind(_) => panic!("Expected Named volume"),
        }

        // Check second named volume with readonly
        match &go_config.volumes[1] {
            VolumeMount::Named(named) => {
                assert_eq!(named.name, "go-build-cache");
                assert_eq!(named.container, "/home/agent/.cache/go-build");
                assert!(named.readonly);
            }
            VolumeMount::Bind(_) => panic!("Expected Named volume"),
        }
    }

    #[test]
    fn test_mixed_volume_config_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[project.mixed-project]
volumes = [
    { host = "~/.cache/shared", container = "/cache" },
    { name = "data-volume", container = "/data" }
]
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        let project_config = config.get_project_config("mixed-project").unwrap();
        assert_eq!(project_config.volumes.len(), 2);

        // First should be bind mount
        match &project_config.volumes[0] {
            VolumeMount::Bind(bind) => {
                assert_eq!(bind.host, "~/.cache/shared");
                assert_eq!(bind.container, "/cache");
            }
            VolumeMount::Named(_) => panic!("Expected Bind mount"),
        }

        // Second should be named volume
        match &project_config.volumes[1] {
            VolumeMount::Named(named) => {
                assert_eq!(named.name, "data-volume");
                assert_eq!(named.container, "/data");
            }
            VolumeMount::Bind(_) => panic!("Expected Named volume"),
        }
    }
}
