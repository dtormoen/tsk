//! User configuration loaded from tsk.toml
//!
//! This module contains configuration types that are loaded from the user's
//! configuration file (`~/.config/tsk/tsk.toml`). These options allow users
//! to customize container resources, project-specific settings, and shared
//! configuration that can be layered via `[defaults]` and `[project.<name>]`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::path::PathBuf;

use super::tsk_env::TskEnvError;

/// User configuration loaded from tsk.toml
///
/// Contains user-configurable options for TSK, including container engine
/// selection, server settings, shared defaults, and project-specific overrides.
///
/// Configuration is resolved via [`TskConfig::resolve_config`] which layers
/// `[defaults]` and `[project.<name>]` sections with proper merge semantics.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TskConfig {
    /// Container engine to use (top-level setting)
    #[serde(default)]
    pub container_engine: ContainerEngine,
    /// Server daemon configuration
    #[serde(default)]
    pub server: ServerConfig,
    /// Shared default configuration applied to all projects
    #[serde(default)]
    pub defaults: SharedConfig,
    /// Project-specific configurations keyed by project name
    #[serde(default)]
    pub project: HashMap<String, SharedConfig>,
}

impl TskConfig {
    /// Resolve configuration for a specific project by layering defaults and project overrides.
    ///
    /// Resolution order (later layers override earlier):
    /// 1. Built-in defaults
    /// 2. `[defaults]` section
    /// 3. Project-level `.tsk/tsk.toml` (if provided)
    /// 4. `[project.<name>]` section (if present)
    pub fn resolve_config(
        &self,
        project_name: &str,
        project_config: Option<&SharedConfig>,
    ) -> ResolvedConfig {
        let mut resolved = ResolvedConfig::default();

        // Apply defaults layer
        self.apply_shared_config(&mut resolved, &self.defaults);

        // Apply project-level .tsk/tsk.toml layer (if provided)
        if let Some(config) = project_config {
            self.apply_shared_config(&mut resolved, config);
        }

        // Apply user [project.<name>] layer (highest priority)
        if let Some(user_project_config) = self.project.get(project_name) {
            self.apply_shared_config(&mut resolved, user_project_config);
        }

        resolved
    }

    /// Apply a SharedConfig layer onto a ResolvedConfig with proper merge semantics.
    ///
    /// - Scalars: override if `Some`
    /// - Lists (`host_services`, `volumes`, `env`): combine with dedup/conflict resolution
    /// - Maps (`stack_config`, `agent_config`): combine keys, same key replaces entire value
    fn apply_shared_config(&self, resolved: &mut ResolvedConfig, config: &SharedConfig) {
        if let Some(ref agent) = config.agent {
            resolved.agent = agent.clone();
        }
        if let Some(ref stack) = config.stack {
            resolved.stack = stack.clone();
        }
        if let Some(dind) = config.dind {
            resolved.dind = dind;
        }
        if let Some(memory) = config.memory_limit_gb {
            resolved.memory_limit_gb = memory;
        }
        if let Some(cpu) = config.cpu_limit {
            resolved.cpu_limit = cpu;
        }
        if let Some(git_town) = config.git_town {
            resolved.git_town = git_town;
        }
        if let Some(ref setup) = config.setup {
            resolved.setup = Some(setup.clone());
        }

        // host_services: combine, deduplicate
        for &port in &config.host_services {
            if !resolved.host_services.contains(&port) {
                resolved.host_services.push(port);
            }
        }

        // volumes: combine, higher-priority wins on container path conflict
        for volume in &config.volumes {
            let container_path = match volume {
                VolumeMount::Bind(b) => &b.container,
                VolumeMount::Named(n) => &n.container,
            };
            resolved.volumes.retain(|v| {
                let existing_path = match v {
                    VolumeMount::Bind(b) => &b.container,
                    VolumeMount::Named(n) => &n.container,
                };
                existing_path != container_path
            });
            resolved.volumes.push(volume.clone());
        }

        // env: combine, higher-priority wins on name conflict
        for env_var in &config.env {
            resolved.env.retain(|e| e.name != env_var.name);
            resolved.env.push(env_var.clone());
        }

        // stack_config: combine names, higher-priority replaces entire struct per name
        for (name, stack_cfg) in &config.stack_config {
            resolved
                .stack_config
                .insert(name.clone(), stack_cfg.clone());
        }

        // agent_config: combine names, higher-priority replaces entire struct per name
        for (name, agent_cfg) in &config.agent_config {
            resolved
                .agent_config
                .insert(name.clone(), agent_cfg.clone());
        }
    }

    /// Returns all unique host service ports across defaults and all projects,
    /// formatted as a comma-separated string for environment variables.
    ///
    /// The proxy container is shared across all tasks, so it needs the union
    /// of all configured host_services. Ports are sorted for deterministic output.
    pub fn all_host_services_env(&self) -> String {
        let mut ports: Vec<u16> = Vec::new();
        for &port in &self.defaults.host_services {
            if !ports.contains(&port) {
                ports.push(port);
            }
        }
        for project in self.project.values() {
            for &port in &project.host_services {
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
        ports.sort();
        ports
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Container engine to use for running containers
#[derive(Debug, Clone, Deserialize, PartialEq, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ContainerEngine {
    Docker,
    Podman,
}

impl Default for ContainerEngine {
    fn default() -> Self {
        if std::env::var("TSK_CONTAINER").is_ok() {
            ContainerEngine::Podman
        } else {
            ContainerEngine::Docker
        }
    }
}

/// Shared configuration shape used by both `[defaults]` and `[project.<name>]` sections.
///
/// All fields are optional so they can be layered during resolution.
/// Lists combine across layers; scalars take the highest-priority `Some` value.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SharedConfig {
    /// Default agent (e.g., "claude", "codex")
    pub agent: Option<String>,
    /// Default stack (e.g., "go", "rust", "python")
    pub stack: Option<String>,
    /// Enable Docker-in-Docker support
    pub dind: Option<bool>,
    /// Container memory limit in gigabytes
    pub memory_limit_gb: Option<f64>,
    /// Number of CPUs available to container
    pub cpu_limit: Option<u32>,
    /// Enable git-town parent branch tracking
    pub git_town: Option<bool>,
    /// Host service ports to forward from proxy to host
    #[serde(default)]
    pub host_services: Vec<u16>,
    /// Custom setup commands for the container
    pub setup: Option<String>,
    /// Per-stack configuration overrides
    #[serde(default)]
    pub stack_config: HashMap<String, StackConfig>,
    /// Per-agent configuration overrides
    #[serde(default)]
    pub agent_config: HashMap<String, AgentConfig>,
    /// Volume mounts for containers
    #[serde(default)]
    pub volumes: Vec<VolumeMount>,
    /// Environment variables for containers
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

/// Per-stack configuration (e.g., custom Dockerfile setup commands)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct StackConfig {
    /// Custom setup commands for this stack layer
    pub setup: Option<String>,
}

/// Per-agent configuration (e.g., custom Dockerfile setup commands)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Custom setup commands for this agent layer
    pub setup: Option<String>,
}

/// Fully resolved configuration with no optional scalars.
///
/// Produced by [`TskConfig::resolve_config`] after layering `[defaults]`
/// and `[project.<name>]` sections over built-in defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedConfig {
    /// Agent to use (default: "claude")
    pub agent: String,
    /// Stack to use (default: "default")
    pub stack: String,
    /// Docker-in-Docker support (default: false)
    pub dind: bool,
    /// Container memory limit in gigabytes (default: 12.0)
    pub memory_limit_gb: f64,
    /// Number of CPUs available to container (default: 8)
    pub cpu_limit: u32,
    /// Git-town parent branch tracking (default: false)
    pub git_town: bool,
    /// Host service ports to forward from proxy
    pub host_services: Vec<u16>,
    /// Custom setup commands
    pub setup: Option<String>,
    /// Per-stack configuration overrides
    pub stack_config: HashMap<String, StackConfig>,
    /// Per-agent configuration overrides
    pub agent_config: HashMap<String, AgentConfig>,
    /// Volume mounts for containers
    pub volumes: Vec<VolumeMount>,
    /// Environment variables for containers
    pub env: Vec<EnvVar>,
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        Self {
            agent: "claude".to_string(),
            stack: "default".to_string(),
            dind: false,
            memory_limit_gb: 12.0,
            cpu_limit: 8,
            git_town: false,
            host_services: Vec::new(),
            setup: None,
            stack_config: HashMap::new(),
            agent_config: HashMap::new(),
            volumes: Vec::new(),
            env: Vec::new(),
        }
    }
}

impl ResolvedConfig {
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

    /// Returns host_services as a comma-separated string for environment variables.
    ///
    /// Returns empty string if no services are configured.
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

    /// Returns true if any host service ports are configured
    pub fn has_host_services(&self) -> bool {
        !self.host_services.is_empty()
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

/// Environment variable configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum VolumeMount {
    /// Bind mount from host filesystem
    Bind(BindMount),
    /// Docker-managed named volume
    Named(NamedVolume),
}

/// Bind mount configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
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
/// Detects old configuration format and prints migration guidance.
pub fn load_config(config_dir: &Path) -> TskConfig {
    let config_file = config_dir.join("tsk.toml");
    if config_file.exists() {
        match std::fs::read_to_string(&config_file) {
            Ok(content) => {
                // Check for old config format
                if let Ok(value) = content.parse::<toml::Value>() {
                    let old_sections: Vec<&str> = ["docker", "proxy", "git_town"]
                        .iter()
                        .filter(|key| value.get(key).is_some())
                        .copied()
                        .collect();
                    if !old_sections.is_empty() {
                        eprintln!(
                            "Error: Your tsk.toml uses the old configuration format.\n\
                             Found deprecated sections: {}\n\n\
                             The configuration format has changed. Please migrate your config:\n\
                             - [docker] settings → top-level `container_engine` and [defaults] section\n\
                             - [proxy] host_services → [defaults] host_services\n\
                             - [git_town] enabled → [defaults] git_town\n\
                             - [project.<name>] fields remain the same but now support all shared config fields\n\n\
                             See the README for the new configuration format.",
                            old_sections.join(", ")
                        );
                        return TskConfig::default();
                    }
                }

                match toml::from_str(&content) {
                    Ok(config) => return config,
                    Err(e) => {
                        eprintln!("Warning: Failed to parse {}: {}", config_file.display(), e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to read {}: {}", config_file.display(), e);
            }
        }
    }
    TskConfig::default()
}

/// Load project-level configuration from `.tsk/tsk.toml` in the project root.
///
/// Returns `None` if the file doesn't exist or can't be parsed (fail-open with warning).
/// The returned [`SharedConfig`] is intended to be passed to [`TskConfig::resolve_config`]
/// as the project config layer.
pub fn load_project_config(project_root: &Path) -> Option<SharedConfig> {
    let config_file = project_root.join(".tsk").join("tsk.toml");
    if config_file.exists() {
        match std::fs::read_to_string(&config_file) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => return Some(config),
                Err(e) => {
                    eprintln!("Warning: Failed to parse {}: {}", config_file.display(), e);
                }
            },
            Err(e) => {
                eprintln!("Warning: Failed to read {}: {}", config_file.display(), e);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_resolved_config_default() {
        let resolved = ResolvedConfig::default();
        assert_eq!(resolved.agent, "claude");
        assert_eq!(resolved.stack, "default");
        assert!(!resolved.dind);
        assert_eq!(resolved.memory_limit_gb, 12.0);
        assert_eq!(resolved.cpu_limit, 8);
        assert!(!resolved.git_town);
        assert!(resolved.host_services.is_empty());
        assert!(resolved.setup.is_none());
        assert!(resolved.stack_config.is_empty());
        assert!(resolved.agent_config.is_empty());
        assert!(resolved.volumes.is_empty());
        assert!(resolved.env.is_empty());
    }

    #[test]
    fn test_resolved_config_conversion_methods() {
        let resolved = ResolvedConfig::default();
        // 12 GB = 12 * 1024 * 1024 * 1024 bytes
        assert_eq!(resolved.memory_limit_bytes(), 12 * 1024 * 1024 * 1024);
        // 8 CPUs = 8 * 100,000 microseconds
        assert_eq!(resolved.cpu_quota_microseconds(), 800_000);

        // Test with custom values
        let custom = ResolvedConfig {
            memory_limit_gb: 5.5,
            cpu_limit: 4,
            host_services: vec![5432, 6379, 3000],
            ..Default::default()
        };
        assert_eq!(
            custom.memory_limit_bytes(),
            (5.5 * 1024.0 * 1024.0 * 1024.0) as i64
        );
        assert_eq!(custom.cpu_quota_microseconds(), 400_000);
        assert_eq!(custom.host_services_env(), "5432,6379,3000");
        assert!(custom.has_host_services());

        // Empty host services
        assert_eq!(resolved.host_services_env(), "");
        assert!(!resolved.has_host_services());
    }

    #[test]
    fn test_tsk_config_default() {
        let config = TskConfig::default();
        assert!(config.project.is_empty());
        assert!(config.defaults.agent.is_none());
        assert!(config.defaults.stack.is_none());
        assert!(config.defaults.host_services.is_empty());
        assert!(config.server.auto_clean_enabled);
        assert_eq!(config.server.auto_clean_age_days, 7.0);
    }

    #[test]
    fn test_config_from_new_toml_format() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
container_engine = "podman"

[server]
auto_clean_enabled = false
auto_clean_age_days = 14.0

[defaults]
memory_limit_gb = 16.0
cpu_limit = 4
host_services = [6379]
git_town = true

[project.my-project]
agent = "codex"
stack = "rust"
memory_limit_gb = 24.0
cpu_limit = 16
dind = true
volumes = [
    { host = "~/debug-logs", container = "/debug", readonly = true }
]
env = [
    { name = "RUST_LOG", value = "debug" }
]
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        assert_eq!(config.container_engine, ContainerEngine::Podman);
        assert!(!config.server.auto_clean_enabled);
        assert_eq!(config.server.auto_clean_age_days, 14.0);

        // Check defaults
        assert_eq!(config.defaults.memory_limit_gb, Some(16.0));
        assert_eq!(config.defaults.cpu_limit, Some(4));
        assert_eq!(config.defaults.host_services, vec![6379]);
        assert_eq!(config.defaults.git_town, Some(true));

        // Check project
        let project = config.project.get("my-project").unwrap();
        assert_eq!(project.agent, Some("codex".to_string()));
        assert_eq!(project.stack, Some("rust".to_string()));
        assert_eq!(project.memory_limit_gb, Some(24.0));
        assert_eq!(project.cpu_limit, Some(16));
        assert_eq!(project.dind, Some(true));
        assert_eq!(project.volumes.len(), 1);
        assert_eq!(project.env.len(), 1);
    }

    #[test]
    fn test_resolve_config_merging_scalars() {
        let config = TskConfig {
            defaults: SharedConfig {
                agent: Some("codex".to_string()),
                memory_limit_gb: Some(16.0),
                git_town: Some(true),
                ..Default::default()
            },
            project: HashMap::from([(
                "my-project".to_string(),
                SharedConfig {
                    agent: Some("claude".to_string()),
                    stack: Some("rust".to_string()),
                    cpu_limit: Some(16),
                    dind: Some(true),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        let resolved = config.resolve_config("my-project", None);

        // Project overrides defaults
        assert_eq!(resolved.agent, "claude");
        // Project sets stack
        assert_eq!(resolved.stack, "rust");
        // Defaults sets memory (project doesn't override)
        assert_eq!(resolved.memory_limit_gb, 16.0);
        // Project sets cpu_limit
        assert_eq!(resolved.cpu_limit, 16);
        // Project sets dind
        assert!(resolved.dind);
        // Defaults sets git_town
        assert!(resolved.git_town);

        // Non-existent project only gets defaults
        let resolved_other = config.resolve_config("other-project", None);
        assert_eq!(resolved_other.agent, "codex");
        assert_eq!(resolved_other.stack, "default");
        assert_eq!(resolved_other.memory_limit_gb, 16.0);
        assert_eq!(resolved_other.cpu_limit, 8); // built-in default
    }

    #[test]
    fn test_resolve_config_merging_host_services() {
        let config = TskConfig {
            defaults: SharedConfig {
                host_services: vec![5432, 6379],
                ..Default::default()
            },
            project: HashMap::from([(
                "my-project".to_string(),
                SharedConfig {
                    host_services: vec![6379, 3000],
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        let resolved = config.resolve_config("my-project", None);

        // Combined and deduplicated
        assert_eq!(resolved.host_services, vec![5432, 6379, 3000]);
    }

    #[test]
    fn test_resolve_config_merging_volumes() {
        let config = TskConfig {
            defaults: SharedConfig {
                volumes: vec![
                    VolumeMount::Bind(BindMount {
                        host: "/host/cache".to_string(),
                        container: "/cache".to_string(),
                        readonly: false,
                    }),
                    VolumeMount::Named(NamedVolume {
                        name: "data".to_string(),
                        container: "/data".to_string(),
                        readonly: false,
                    }),
                ],
                ..Default::default()
            },
            project: HashMap::from([(
                "my-project".to_string(),
                SharedConfig {
                    volumes: vec![
                        // Override /cache with a named volume (different type, same container path)
                        VolumeMount::Named(NamedVolume {
                            name: "project-cache".to_string(),
                            container: "/cache".to_string(),
                            readonly: true,
                        }),
                        // New volume
                        VolumeMount::Bind(BindMount {
                            host: "/host/logs".to_string(),
                            container: "/logs".to_string(),
                            readonly: true,
                        }),
                    ],
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        let resolved = config.resolve_config("my-project", None);

        // /data from defaults remains, /cache replaced by project's named volume, /logs added
        assert_eq!(resolved.volumes.len(), 3);

        match &resolved.volumes[0] {
            VolumeMount::Named(n) => {
                assert_eq!(n.name, "data");
                assert_eq!(n.container, "/data");
            }
            _ => panic!("Expected Named volume for /data"),
        }

        match &resolved.volumes[1] {
            VolumeMount::Named(n) => {
                assert_eq!(n.name, "project-cache");
                assert_eq!(n.container, "/cache");
                assert!(n.readonly);
            }
            _ => panic!("Expected Named volume for /cache"),
        }

        match &resolved.volumes[2] {
            VolumeMount::Bind(b) => {
                assert_eq!(b.host, "/host/logs");
                assert_eq!(b.container, "/logs");
                assert!(b.readonly);
            }
            _ => panic!("Expected Bind mount for /logs"),
        }
    }

    #[test]
    fn test_resolve_config_merging_env() {
        let config = TskConfig {
            defaults: SharedConfig {
                env: vec![
                    EnvVar {
                        name: "DATABASE_URL".to_string(),
                        value: "postgres://localhost/db".to_string(),
                    },
                    EnvVar {
                        name: "DEBUG".to_string(),
                        value: "false".to_string(),
                    },
                ],
                ..Default::default()
            },
            project: HashMap::from([(
                "my-project".to_string(),
                SharedConfig {
                    env: vec![
                        EnvVar {
                            name: "DEBUG".to_string(),
                            value: "true".to_string(),
                        },
                        EnvVar {
                            name: "RUST_LOG".to_string(),
                            value: "info".to_string(),
                        },
                    ],
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        let resolved = config.resolve_config("my-project", None);

        assert_eq!(resolved.env.len(), 3);
        assert_eq!(resolved.env[0].name, "DATABASE_URL");
        assert_eq!(resolved.env[0].value, "postgres://localhost/db");
        assert_eq!(resolved.env[1].name, "DEBUG");
        assert_eq!(resolved.env[1].value, "true");
        assert_eq!(resolved.env[2].name, "RUST_LOG");
        assert_eq!(resolved.env[2].value, "info");
    }

    #[test]
    fn test_resolve_config_merging_stack_config() {
        let config = TskConfig {
            defaults: SharedConfig {
                stack_config: HashMap::from([
                    (
                        "rust".to_string(),
                        StackConfig {
                            setup: Some("RUN apt-get install -y cmake".to_string()),
                        },
                    ),
                    (
                        "go".to_string(),
                        StackConfig {
                            setup: Some("RUN go install tool".to_string()),
                        },
                    ),
                ]),
                ..Default::default()
            },
            project: HashMap::from([(
                "my-project".to_string(),
                SharedConfig {
                    stack_config: HashMap::from([
                        (
                            "rust".to_string(),
                            StackConfig {
                                setup: Some("RUN cargo install custom-tool".to_string()),
                            },
                        ),
                        (
                            "java".to_string(),
                            StackConfig {
                                setup: Some("RUN apt-get install -y openjdk-17-jdk".to_string()),
                            },
                        ),
                    ]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        let resolved = config.resolve_config("my-project", None);

        assert_eq!(resolved.stack_config.len(), 3);
        assert_eq!(
            resolved.stack_config["rust"].setup,
            Some("RUN cargo install custom-tool".to_string())
        );
        assert_eq!(
            resolved.stack_config["go"].setup,
            Some("RUN go install tool".to_string())
        );
        assert_eq!(
            resolved.stack_config["java"].setup,
            Some("RUN apt-get install -y openjdk-17-jdk".to_string())
        );
    }

    #[test]
    fn test_resolve_config_merging_agent_config() {
        let config = TskConfig {
            defaults: SharedConfig {
                agent_config: HashMap::from([(
                    "claude".to_string(),
                    AgentConfig {
                        setup: Some("RUN npm install -g tool".to_string()),
                    },
                )]),
                ..Default::default()
            },
            project: HashMap::from([(
                "my-project".to_string(),
                SharedConfig {
                    agent_config: HashMap::from([
                        (
                            "claude".to_string(),
                            AgentConfig {
                                setup: Some("RUN pip install custom".to_string()),
                            },
                        ),
                        (
                            "codex".to_string(),
                            AgentConfig {
                                setup: Some("RUN npm install -g codex-tool".to_string()),
                            },
                        ),
                    ]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        let resolved = config.resolve_config("my-project", None);

        assert_eq!(resolved.agent_config.len(), 2);
        assert_eq!(
            resolved.agent_config["claude"].setup,
            Some("RUN pip install custom".to_string())
        );
        assert_eq!(
            resolved.agent_config["codex"].setup,
            Some("RUN npm install -g codex-tool".to_string())
        );
    }

    #[test]
    fn test_old_format_detection() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        // Config with old [docker] section
        let toml_content = r#"
[docker]
memory_limit_gb = 8.0
cpu_limit = 4
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);
        // Should return defaults since old format is detected
        assert!(config.defaults.memory_limit_gb.is_none());
        assert!(config.defaults.cpu_limit.is_none());

        // Test with [proxy]
        let toml_content = r#"
[proxy]
host_services = [5432]
"#;
        std::fs::write(config_dir.join("tsk.toml"), toml_content).unwrap();
        let config = load_config(config_dir);
        assert!(config.defaults.host_services.is_empty());

        // Test with [git_town]
        let toml_content = r#"
[git_town]
enabled = true
"#;
        std::fs::write(config_dir.join("tsk.toml"), toml_content).unwrap();
        let config = load_config(config_dir);
        assert!(config.defaults.git_town.is_none());
    }

    #[test]
    fn test_config_missing_toml_uses_defaults() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config = load_config(temp_dir.path());

        let resolved = config.resolve_config("any-project", None);
        assert_eq!(resolved.agent, "claude");
        assert_eq!(resolved.stack, "default");
        assert_eq!(resolved.memory_limit_gb, 12.0);
        assert_eq!(resolved.cpu_limit, 8);
        assert!(!resolved.dind);
        assert!(!resolved.git_town);
    }

    #[test]
    fn test_bind_mount_path_expansion() {
        let bind_mount = BindMount {
            host: "~/.cache/go-build".to_string(),
            container: "/home/agent/.cache/go-build".to_string(),
            readonly: false,
        };

        let expanded = bind_mount.expanded_host_path().unwrap();
        assert!(!expanded.to_string_lossy().starts_with("~"));
        assert!(expanded.to_string_lossy().ends_with(".cache/go-build"));

        let bind_mount_home = BindMount {
            host: "~".to_string(),
            container: "/home/agent".to_string(),
            readonly: false,
        };

        let expanded_home = bind_mount_home.expanded_host_path().unwrap();
        assert!(!expanded_home.to_string_lossy().starts_with("~"));
        assert!(!expanded_home.to_string_lossy().is_empty());

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

        let go_config = config.project.get("my-go-project").unwrap();
        assert_eq!(go_config.volumes.len(), 2);

        match &go_config.volumes[0] {
            VolumeMount::Named(named) => {
                assert_eq!(named.name, "go-mod-cache");
                assert_eq!(named.container, "/go/pkg/mod");
                assert!(!named.readonly);
            }
            VolumeMount::Bind(_) => panic!("Expected Named volume"),
        }

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

        let project_config = config.project.get("mixed-project").unwrap();
        assert_eq!(project_config.volumes.len(), 2);

        match &project_config.volumes[0] {
            VolumeMount::Bind(bind) => {
                assert_eq!(bind.host, "~/.cache/shared");
                assert_eq!(bind.container, "/cache");
            }
            VolumeMount::Named(_) => panic!("Expected Bind mount"),
        }

        match &project_config.volumes[1] {
            VolumeMount::Named(named) => {
                assert_eq!(named.name, "data-volume");
                assert_eq!(named.container, "/data");
            }
            VolumeMount::Bind(_) => panic!("Expected Named volume"),
        }
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
    fn test_container_engine_default_depends_on_environment() {
        let config = TskConfig::default();
        if std::env::var("TSK_CONTAINER").is_ok() {
            assert_eq!(config.container_engine, ContainerEngine::Podman);
        } else {
            assert_eq!(config.container_engine, ContainerEngine::Docker);
        }
    }

    #[test]
    fn test_stack_config_parsing() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[defaults.stack_config.scala]
setup = "RUN apt-get install -y scala"

[defaults.stack_config.rust]
setup = "RUN apt-get install -y cmake"

[project.my-project.stack_config.java]
setup = "RUN apt-get install -y openjdk-17-jdk"
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        assert_eq!(config.defaults.stack_config.len(), 2);
        assert_eq!(
            config.defaults.stack_config["scala"].setup,
            Some("RUN apt-get install -y scala".to_string())
        );
        assert_eq!(
            config.defaults.stack_config["rust"].setup,
            Some("RUN apt-get install -y cmake".to_string())
        );

        let project = config.project.get("my-project").unwrap();
        assert_eq!(project.stack_config.len(), 1);
        assert_eq!(
            project.stack_config["java"].setup,
            Some("RUN apt-get install -y openjdk-17-jdk".to_string())
        );
    }

    #[test]
    fn test_agent_config_parsing() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_dir = temp_dir.path();

        let toml_content = r#"
[defaults.agent_config.claude]
setup = "RUN npm install -g tool"

[project.my-project.agent_config.my-agent]
setup = "RUN pip install custom-tool"
"#;
        let mut file = std::fs::File::create(config_dir.join("tsk.toml")).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = load_config(config_dir);

        assert_eq!(config.defaults.agent_config.len(), 1);
        assert_eq!(
            config.defaults.agent_config["claude"].setup,
            Some("RUN npm install -g tool".to_string())
        );

        let project = config.project.get("my-project").unwrap();
        assert_eq!(project.agent_config.len(), 1);
        assert_eq!(
            project.agent_config["my-agent"].setup,
            Some("RUN pip install custom-tool".to_string())
        );
    }

    #[test]
    fn test_load_project_config() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let project_root = temp_dir.path();

        std::fs::create_dir_all(project_root.join(".tsk")).unwrap();
        let toml_content = r#"
agent = "codex"
stack = "python"
memory_limit_gb = 20.0
host_services = [8080]
setup = "RUN pip install custom-tool"

[stack_config.python]
setup = "RUN pip install numpy"
"#;
        std::fs::write(project_root.join(".tsk/tsk.toml"), toml_content).unwrap();

        let config = load_project_config(project_root).unwrap();
        assert_eq!(config.agent, Some("codex".to_string()));
        assert_eq!(config.stack, Some("python".to_string()));
        assert_eq!(config.memory_limit_gb, Some(20.0));
        assert_eq!(config.host_services, vec![8080]);
        assert_eq!(
            config.setup,
            Some("RUN pip install custom-tool".to_string())
        );
        assert_eq!(
            config.stack_config["python"].setup,
            Some("RUN pip install numpy".to_string())
        );
    }

    #[test]
    fn test_load_project_config_missing() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        assert!(load_project_config(temp_dir.path()).is_none());
    }

    #[test]
    fn test_resolve_config_with_project_config_priority() {
        // Test the full 4-layer chain:
        // built-in < defaults < project .tsk/tsk.toml < user [project.<name>]
        let project_config = SharedConfig {
            agent: Some("codex".to_string()),
            memory_limit_gb: Some(20.0),
            cpu_limit: Some(12),
            host_services: vec![8080],
            env: vec![EnvVar {
                name: "PROJECT_VAR".to_string(),
                value: "from-project-file".to_string(),
            }],
            stack_config: HashMap::from([(
                "python".to_string(),
                StackConfig {
                    setup: Some("RUN pip install project-dep".to_string()),
                },
            )]),
            ..Default::default()
        };

        let config = TskConfig {
            defaults: SharedConfig {
                memory_limit_gb: Some(16.0),
                git_town: Some(true),
                host_services: vec![5432],
                env: vec![
                    EnvVar {
                        name: "DEFAULT_VAR".to_string(),
                        value: "from-defaults".to_string(),
                    },
                    EnvVar {
                        name: "PROJECT_VAR".to_string(),
                        value: "from-defaults".to_string(),
                    },
                ],
                ..Default::default()
            },
            project: HashMap::from([(
                "my-project".to_string(),
                SharedConfig {
                    agent: Some("claude".to_string()),
                    cpu_limit: Some(16),
                    host_services: vec![6379],
                    env: vec![EnvVar {
                        name: "USER_VAR".to_string(),
                        value: "from-user-project".to_string(),
                    }],
                    stack_config: HashMap::from([(
                        "python".to_string(),
                        StackConfig {
                            setup: Some("RUN pip install user-dep".to_string()),
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        let resolved = config.resolve_config("my-project", Some(&project_config));

        // user [project] overrides project config
        assert_eq!(resolved.agent, "claude");
        // project config overrides defaults
        assert_eq!(resolved.memory_limit_gb, 20.0);
        // user [project] overrides project config
        assert_eq!(resolved.cpu_limit, 16);
        // defaults (no override from project or user project)
        assert!(resolved.git_town);
        // host_services combined from all layers, deduplicated
        assert!(resolved.host_services.contains(&5432));
        assert!(resolved.host_services.contains(&8080));
        assert!(resolved.host_services.contains(&6379));
        // env: project config overrides PROJECT_VAR from defaults
        assert!(
            resolved
                .env
                .iter()
                .any(|e| e.name == "DEFAULT_VAR" && e.value == "from-defaults")
        );
        assert!(
            resolved
                .env
                .iter()
                .any(|e| e.name == "PROJECT_VAR" && e.value == "from-project-file")
        );
        assert!(
            resolved
                .env
                .iter()
                .any(|e| e.name == "USER_VAR" && e.value == "from-user-project")
        );
        // stack_config: user [project] replaces python setup
        assert_eq!(
            resolved.stack_config["python"].setup,
            Some("RUN pip install user-dep".to_string())
        );
    }

    #[test]
    fn test_resolve_config_project_config_without_user_project() {
        // When there's no user [project.<name>] section, project config should still work
        let project_config = SharedConfig {
            agent: Some("codex".to_string()),
            stack: Some("python".to_string()),
            memory_limit_gb: Some(20.0),
            ..Default::default()
        };

        let config = TskConfig {
            defaults: SharedConfig {
                memory_limit_gb: Some(16.0),
                cpu_limit: Some(4),
                ..Default::default()
            },
            ..Default::default()
        };

        let resolved = config.resolve_config("my-project", Some(&project_config));

        // project config overrides defaults for agent and memory
        assert_eq!(resolved.agent, "codex");
        assert_eq!(resolved.stack, "python");
        assert_eq!(resolved.memory_limit_gb, 20.0);
        // defaults still apply for unset fields
        assert_eq!(resolved.cpu_limit, 4);
    }

    #[test]
    fn test_all_host_services_env() {
        // Empty config
        let config = TskConfig::default();
        assert_eq!(config.all_host_services_env(), "");

        // Services only in defaults
        let config = TskConfig {
            defaults: SharedConfig {
                host_services: vec![5432, 6379],
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.all_host_services_env(), "5432,6379");

        // Services only in project
        let config = TskConfig {
            project: HashMap::from([(
                "my-project".to_string(),
                SharedConfig {
                    host_services: vec![3000],
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        assert_eq!(config.all_host_services_env(), "3000");

        // Services in both defaults and project, with dedup
        let config = TskConfig {
            defaults: SharedConfig {
                host_services: vec![5432, 6379],
                ..Default::default()
            },
            project: HashMap::from([(
                "project-a".to_string(),
                SharedConfig {
                    host_services: vec![6379, 3000],
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let result = config.all_host_services_env();
        assert_eq!(result, "3000,5432,6379");
    }

    #[test]
    fn test_resolved_config_json_round_trip() {
        let config = ResolvedConfig {
            agent: "codex".to_string(),
            stack: "rust".to_string(),
            dind: true,
            memory_limit_gb: 24.0,
            cpu_limit: 16,
            git_town: true,
            host_services: vec![5432, 6379],
            setup: Some("RUN apt-get install -y cmake".to_string()),
            stack_config: HashMap::from([(
                "rust".to_string(),
                StackConfig {
                    setup: Some("RUN cargo install nextest".to_string()),
                },
            )]),
            agent_config: HashMap::from([(
                "codex".to_string(),
                AgentConfig {
                    setup: Some("RUN pip install tool".to_string()),
                },
            )]),
            volumes: vec![
                VolumeMount::Bind(BindMount {
                    host: "/host/path".to_string(),
                    container: "/container/path".to_string(),
                    readonly: true,
                }),
                VolumeMount::Named(NamedVolume {
                    name: "cache".to_string(),
                    container: "/cache".to_string(),
                    readonly: false,
                }),
            ],
            env: vec![EnvVar {
                name: "DB_URL".to_string(),
                value: "postgres://localhost/db".to_string(),
            }],
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ResolvedConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.agent, "codex");
        assert_eq!(deserialized.stack, "rust");
        assert!(deserialized.dind);
        assert_eq!(deserialized.memory_limit_gb, 24.0);
        assert_eq!(deserialized.cpu_limit, 16);
        assert!(deserialized.git_town);
        assert_eq!(deserialized.host_services, vec![5432, 6379]);
        assert_eq!(
            deserialized.setup,
            Some("RUN apt-get install -y cmake".to_string())
        );
        assert_eq!(deserialized.stack_config.len(), 1);
        assert_eq!(
            deserialized.stack_config["rust"].setup,
            Some("RUN cargo install nextest".to_string())
        );
        assert_eq!(deserialized.agent_config.len(), 1);
        assert_eq!(deserialized.volumes.len(), 2);
        assert_eq!(deserialized.env.len(), 1);
        assert_eq!(deserialized.env[0].name, "DB_URL");
    }
}
