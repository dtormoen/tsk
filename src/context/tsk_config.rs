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
    /// Git-town integration configuration
    #[serde(default)]
    pub git_town: GitTownConfig,
    /// Proxy configuration for host service access
    #[serde(default)]
    pub proxy: ProxyConfig,
    /// Server daemon configuration
    #[serde(default)]
    pub server: ServerConfig,
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

    /// Returns true if any host service ports are configured for proxy forwarding
    pub fn has_host_services(&self) -> bool {
        !self.proxy.host_services.is_empty()
    }
}

/// Container engine to use for running containers
#[derive(Debug, Clone, Default, Deserialize, PartialEq, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ContainerEngine {
    #[default]
    Docker,
    Podman,
}

/// Docker container resource configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DockerOptions {
    /// Container engine to use (default: docker)
    #[serde(default)]
    pub container_engine: ContainerEngine,
    /// Container memory limit in gigabytes (default: 12.0)
    pub memory_limit_gb: f64,
    /// Number of CPUs available to container (default: 8)
    pub cpu_limit: u32,
}

impl Default for DockerOptions {
    fn default() -> Self {
        let container_engine = if std::env::var("TSK_CONTAINER").is_ok() {
            ContainerEngine::Podman
        } else {
            ContainerEngine::default()
        };
        Self {
            container_engine,
            memory_limit_gb: 12.0, // 12GB
            cpu_limit: 8,          // 8 CPUs
        }
    }
}

impl DockerOptions {
    /// Convert memory limit from gigabytes to bytes for Docker/Bollard API
    pub fn memory_limit_bytes(&self) -> i64 {
        (self.memory_limit_gb * 1024.0 * 1024.0 * 1024.0) as i64
    }

    /// Convert CPU limit to microseconds per 100ms period for Docker/Bollard API
    ///
    /// Docker uses cpu_quota to limit CPU usage. The value represents microseconds
    /// per 100ms period (cpu_period defaults to 100000 microseconds).
    /// So 100,000 = 1 CPU, 200,000 = 2 CPUs, etc.
    pub fn cpu_quota_microseconds(&self) -> i64 {
        self.cpu_limit as i64 * 100_000
    }
}

/// Git-town integration configuration
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GitTownConfig {
    /// Enable git-town parent branch tracking
    ///
    /// When enabled, TSK will set the git-town parent branch configuration
    /// for task branches, using the branch that was checked out when the
    /// task was created as the parent.
    pub enabled: bool,
}

/// Proxy configuration for network isolation and host service access
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    /// Host service ports to forward from proxy to host.docker.internal
    /// Agents connect to tsk-proxy:<port> to access these services
    #[serde(default)]
    pub host_services: Vec<u16>,
}

impl ProxyConfig {
    /// Returns host_services as a comma-separated string for environment variables
    /// Returns empty string if no services are configured
    pub fn host_services_env(&self) -> String {
        if self.host_services.is_empty() {
            String::new()
        } else {
            self.host_services
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(",")
        }
    }
}

/// Server daemon configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Enable automatic cleanup of old completed/failed tasks (default: true)
    pub auto_clean_enabled: bool,
    /// Minimum age in days before a task is eligible for auto-cleanup (default: 7.0)
    ///
    /// Supports fractional days (e.g., 0.5 for 12 hours). Negative values are
    /// clamped to 0.
    pub auto_clean_age_days: f64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            auto_clean_enabled: true,
            auto_clean_age_days: 7.0,
        }
    }
}

impl ServerConfig {
    /// Convert `auto_clean_age_days` to a `chrono::Duration`.
    ///
    /// Negative values are clamped to zero (clean immediately on next cycle).
    pub fn auto_clean_min_age(&self) -> chrono::Duration {
        let days = f64::max(0.0, self.auto_clean_age_days);
        let seconds = (days * 86_400.0) as i64;
        chrono::Duration::seconds(seconds)
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
    /// Environment variables for Docker containers
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

/// Environment variable configuration
#[derive(Debug, Clone, Deserialize)]
pub struct EnvVar {
    /// Environment variable name
    pub name: String,
    /// Environment variable value
    pub value: String,
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
        assert_eq!(options.memory_limit_gb, 12.0);
        assert_eq!(options.cpu_limit, 8);
    }

    #[test]
    fn test_docker_options_conversion_methods() {
        let options = DockerOptions::default();
        // 12 GB = 12 * 1024 * 1024 * 1024 bytes
        assert_eq!(options.memory_limit_bytes(), 12 * 1024 * 1024 * 1024);
        // 8 CPUs = 8 * 100,000 microseconds
        assert_eq!(options.cpu_quota_microseconds(), 800_000);

        // Test with custom values
        let custom_options = DockerOptions {
            memory_limit_gb: 5.5,
            cpu_limit: 4,
            ..Default::default()
        };
        // 5.5 GB in bytes
        assert_eq!(
            custom_options.memory_limit_bytes(),
            (5.5 * 1024.0 * 1024.0 * 1024.0) as i64
        );
        // 4 CPUs = 400,000 microseconds
        assert_eq!(custom_options.cpu_quota_microseconds(), 400_000);
    }

    #[test]
    fn test_tsk_config_default() {
        let config = TskConfig::default();
        assert_eq!(config.docker.memory_limit_gb, 12.0);
        assert_eq!(config.docker.cpu_limit, 8);
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
memory_limit_gb = 8.0
cpu_limit = 4
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        assert_eq!(config.docker.memory_limit_gb, 8.0);
        assert_eq!(config.docker.cpu_limit, 4);
        // Verify conversion methods work correctly
        assert_eq!(config.docker.memory_limit_bytes(), 8 * 1024 * 1024 * 1024);
        assert_eq!(config.docker.cpu_quota_microseconds(), 400_000);
    }

    #[test]
    fn test_config_partial_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        // Only specify memory_limit_gb, cpu_limit should use default
        let toml_content = r#"
[docker]
memory_limit_gb = 4.0
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        assert_eq!(config.docker.memory_limit_gb, 4.0);
        assert_eq!(config.docker.cpu_limit, 8); // Default 8 CPUs
    }

    #[test]
    fn test_config_missing_toml_uses_defaults() {
        let temp_dir = tempfile::TempDir::new().unwrap();

        let config = load_config(temp_dir.path());

        // Should use defaults when no config file exists
        assert_eq!(config.docker.memory_limit_gb, 12.0);
        assert_eq!(config.docker.cpu_limit, 8);
    }

    #[test]
    fn test_config_invalid_toml_type_uses_defaults() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        // Invalid type: string instead of f64
        let toml_content = r#"
[docker]
memory_limit_gb = "not-a-number"
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        // Should use defaults when TOML contains invalid types
        assert_eq!(config.docker.memory_limit_gb, 12.0);
        assert_eq!(config.docker.cpu_limit, 8);
    }

    #[test]
    fn test_project_config_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[docker]
memory_limit_gb = 8.0

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
    fn test_project_env_vars_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[project.my-app]
agent = "claude"
stack = "go"
env = [
    { name = "DATABASE_URL", value = "postgres://tsk-proxy:5432/mydb" },
    { name = "REDIS_URL", value = "redis://tsk-proxy:6379" },
    { name = "DEBUG", value = "true" },
]
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        let app_config = config.get_project_config("my-app");
        assert!(app_config.is_some());
        let app_config = app_config.unwrap();

        assert_eq!(app_config.env.len(), 3);
        assert_eq!(app_config.env[0].name, "DATABASE_URL");
        assert_eq!(app_config.env[0].value, "postgres://tsk-proxy:5432/mydb");
        assert_eq!(app_config.env[1].name, "REDIS_URL");
        assert_eq!(app_config.env[1].value, "redis://tsk-proxy:6379");
        assert_eq!(app_config.env[2].name, "DEBUG");
        assert_eq!(app_config.env[2].value, "true");
    }

    #[test]
    fn test_project_config_empty_env_by_default() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[project.my-app]
agent = "claude"
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        let app_config = config.get_project_config("my-app").unwrap();
        assert!(app_config.env.is_empty());
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

    #[test]
    fn test_git_town_config_default() {
        let config = GitTownConfig::default();
        assert!(!config.enabled);
    }

    #[test]
    fn test_tsk_config_default_has_disabled_git_town() {
        let config = TskConfig::default();
        assert!(!config.git_town.enabled);
    }

    #[test]
    fn test_git_town_config_enabled_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[git_town]
enabled = true
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);
        assert!(config.git_town.enabled);
    }

    #[test]
    fn test_git_town_config_disabled_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[git_town]
enabled = false
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);
        assert!(!config.git_town.enabled);
    }

    #[test]
    fn test_git_town_config_missing_uses_default() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        // Config with other sections but no git_town
        let toml_content = r#"
[docker]
memory_limit_gb = 8.0
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);
        // Should use default (disabled)
        assert!(!config.git_town.enabled);
    }

    #[test]
    fn test_proxy_config_default() {
        let config = ProxyConfig::default();
        assert!(config.host_services.is_empty());
        assert_eq!(config.host_services_env(), "");
    }

    #[test]
    fn test_proxy_config_host_services_env() {
        let config = ProxyConfig {
            host_services: vec![5432, 6379, 3000],
        };
        assert_eq!(config.host_services_env(), "5432,6379,3000");
    }

    #[test]
    fn test_proxy_config_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[proxy]
host_services = [5432, 6379, 3000]

[docker]
memory_limit_gb = 8.0
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        assert_eq!(config.proxy.host_services, vec![5432, 6379, 3000]);
        assert_eq!(config.proxy.host_services_env(), "5432,6379,3000");
    }

    #[test]
    fn test_tsk_config_default_has_empty_proxy() {
        let config = TskConfig::default();
        assert!(config.proxy.host_services.is_empty());
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert!(config.auto_clean_enabled);
        assert_eq!(config.auto_clean_age_days, 7.0);
    }

    #[test]
    fn test_server_config_auto_clean_min_age() {
        let config = ServerConfig::default();
        assert_eq!(config.auto_clean_min_age(), chrono::Duration::days(7));

        let custom = ServerConfig {
            auto_clean_enabled: true,
            auto_clean_age_days: 0.5,
        };
        assert_eq!(
            custom.auto_clean_min_age(),
            chrono::Duration::seconds(43200)
        );
    }

    #[test]
    fn test_server_config_negative_days_clamped() {
        let config = ServerConfig {
            auto_clean_enabled: true,
            auto_clean_age_days: -5.0,
        };
        assert_eq!(config.auto_clean_min_age(), chrono::Duration::zero());
    }

    #[test]
    fn test_server_config_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[server]
auto_clean_enabled = false
auto_clean_age_days = 14.0
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);
        assert!(!config.server.auto_clean_enabled);
        assert_eq!(config.server.auto_clean_age_days, 14.0);
    }

    #[test]
    fn test_server_config_missing_uses_default() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[docker]
memory_limit_gb = 8.0
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);
        assert!(config.server.auto_clean_enabled);
        assert_eq!(config.server.auto_clean_age_days, 7.0);
    }

    #[test]
    fn test_tsk_config_default_has_server_defaults() {
        let config = TskConfig::default();
        assert!(config.server.auto_clean_enabled);
        assert_eq!(config.server.auto_clean_age_days, 7.0);
    }

    #[test]
    fn test_container_engine_podman_from_toml() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[docker]
container_engine = "podman"
memory_limit_gb = 8.0
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        assert_eq!(config.docker.container_engine, ContainerEngine::Podman);
        assert_eq!(config.docker.memory_limit_gb, 8.0);
    }

    #[test]
    fn test_container_engine_default_depends_on_environment() {
        let config = TskConfig::default();
        if std::env::var("TSK_CONTAINER").is_ok() {
            assert_eq!(config.docker.container_engine, ContainerEngine::Podman);
        } else {
            assert_eq!(config.docker.container_engine, ContainerEngine::Docker);
        }
    }
}
