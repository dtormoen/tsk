use std::env;
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::process::Command;
use thiserror::Error;

use crate::container::{ContainerEngine, EngineConfig, default_socket_path, detect_engine};

#[derive(Debug, Error)]
pub enum TskConfigError {
    #[error("Failed to determine home directory")]
    NoHomeDirectory,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Git configuration error: {0}")]
    GitConfig(String),
    #[error("Failed to parse configuration: {0}")]
    ConfigParse(String),
    #[error("Neither Docker nor Podman found. Please install one.")]
    NoContainerEngine,
}

/// Configuration file structure for config.toml
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub engine: EngineSection,
}

/// Engine configuration section in config.toml
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct EngineSection {
    /// The configured container engine (docker or podman)
    pub name: Option<String>,
    /// The last-used engine (for orphan cleanup detection)
    pub last_used: Option<String>,
    /// Optional custom socket path
    pub socket_path: Option<String>,
}

/// Provides access to XDG Base Directory compliant paths for TSK
#[derive(Debug, Clone)]
pub struct TskConfig {
    data_dir: PathBuf,
    runtime_dir: PathBuf,
    config_dir: PathBuf,
    claude_config_dir: PathBuf,
    codex_config_dir: PathBuf,
    editor: String,
    terminal_type: Option<String>,
    git_user_name: String,
    git_user_email: String,
}

impl TskConfig {
    /// Create new TSK configuration instance with default paths from environment
    ///
    /// Uses XDG Base Directory specification:
    /// - XDG_DATA_HOME for data directory (defaults to ~/.local/share/tsk)
    /// - XDG_RUNTIME_DIR for runtime directory (defaults to /tmp/tsk-$UID)
    /// - XDG_CONFIG_HOME for config directory (defaults to ~/.config/tsk)
    pub fn new() -> Result<Self, TskConfigError> {
        let data_dir = Self::resolve_data_dir(None)?;
        let runtime_dir = Self::resolve_runtime_dir(None)?;
        let config_dir = Self::resolve_config_dir(None)?;
        let claude_config_dir = Self::resolve_claude_config_dir(None)?;
        let codex_config_dir = Self::resolve_codex_config_dir(None)?;
        let editor = Self::resolve_editor(None);
        let terminal_type = Self::resolve_terminal_type(None);
        let git_user_name = Self::resolve_git_user_name(None)?;
        let git_user_email = Self::resolve_git_user_email(None)?;

        Ok(Self {
            data_dir,
            runtime_dir,
            config_dir,
            claude_config_dir,
            codex_config_dir,
            editor,
            terminal_type,
            git_user_name,
            git_user_email,
        })
    }

    /// Create a builder for TskConfig with custom overrides
    #[cfg(test)]
    pub fn builder() -> TskConfigBuilder {
        TskConfigBuilder::new()
    }

    /// Get the data directory path (for persistent storage)
    ///
    /// Used in tests for accessing task storage directory
    #[cfg(test)]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Get the runtime directory path (for sockets, pid files)
    #[cfg(test)]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// Get the config directory path (for configuration files)
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Gets the Claude configuration directory path
    pub fn claude_config_dir(&self) -> &Path {
        &self.claude_config_dir
    }

    /// Gets the Codex configuration directory path
    pub fn codex_config_dir(&self) -> &Path {
        &self.codex_config_dir
    }

    /// Gets the editor command
    pub fn editor(&self) -> &str {
        &self.editor
    }

    /// Gets the terminal type if set
    pub fn terminal_type(&self) -> Option<&str> {
        self.terminal_type.as_deref()
    }

    /// Gets the git user name for Docker builds
    pub fn git_user_name(&self) -> &str {
        &self.git_user_name
    }

    /// Gets the git user email for Docker builds
    pub fn git_user_email(&self) -> &str {
        &self.git_user_email
    }

    /// Get the path to the tasks.json file
    pub fn tasks_file(&self) -> PathBuf {
        self.data_dir.join("tasks.json")
    }

    /// Get the path to a task's directory
    pub fn task_dir(&self, task_id: &str, repo_hash: &str) -> PathBuf {
        self.data_dir
            .join("tasks")
            .join(format!("{repo_hash}-{task_id}"))
    }

    /// Get the server socket path
    pub fn socket_path(&self) -> PathBuf {
        self.runtime_dir.join("tsk.sock")
    }

    /// Get the server PID file path
    pub fn pid_file(&self) -> PathBuf {
        self.runtime_dir.join("tsk.pid")
    }

    /// Ensure all required directories exist
    pub fn ensure_directories(&self) -> Result<(), TskConfigError> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(self.data_dir.join("tasks"))?;
        std::fs::create_dir_all(&self.runtime_dir)?;
        std::fs::create_dir_all(&self.config_dir)?;
        Ok(())
    }

    /// Get the path to the config.toml file
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Load the configuration file
    pub fn load_config_file(&self) -> Result<ConfigFile, TskConfigError> {
        let config_path = self.config_file();
        if !config_path.exists() {
            return Ok(ConfigFile::default());
        }
        let content = std::fs::read_to_string(&config_path)
            .map_err(TskConfigError::Io)?;
        toml::from_str(&content)
            .map_err(|e| TskConfigError::ConfigParse(e.to_string()))
    }

    /// Save the configuration file
    pub fn save_config_file(&self, config: &ConfigFile) -> Result<(), TskConfigError> {
        let config_path = self.config_file();
        let content = toml::to_string_pretty(config)
            .map_err(|e| TskConfigError::ConfigParse(e.to_string()))?;
        std::fs::write(&config_path, content)
            .map_err(TskConfigError::Io)
    }

    /// Resolve the container engine configuration
    ///
    /// Priority order:
    /// 1. CLI override (if provided)
    /// 2. Config file setting
    /// 3. Auto-detection (prefer Podman)
    ///
    /// Returns the engine config and whether a fallback was used
    pub fn resolve_engine_config(
        &self,
        cli_override: Option<ContainerEngine>,
    ) -> Result<(EngineConfig, bool), TskConfigError> {
        let mut config_file = self.load_config_file()?;
        let mut used_fallback = false;

        // Determine the engine to use
        let engine = if let Some(engine) = cli_override {
            // CLI override takes precedence
            engine
        } else if let Some(ref name) = config_file.engine.name {
            // Use configured engine
            name.parse::<ContainerEngine>()
                .map_err(|e| TskConfigError::ConfigParse(e))?
        } else {
            // Auto-detect and persist
            let detected = detect_engine()
                .ok_or_else(|| TskConfigError::NoContainerEngine)?;

            println!("Detected {}, saving to config...", detected);
            config_file.engine.name = Some(detected.to_string());
            config_file.engine.last_used = Some(detected.to_string());
            self.save_config_file(&config_file)?;

            detected
        };

        // Resolve socket path
        let socket_path = if let Some(ref path) = config_file.engine.socket_path {
            PathBuf::from(path)
        } else {
            // Check if configured engine is available
            if let Some(path) = default_socket_path(engine) {
                path
            } else {
                // Engine not available, try fallback
                let other_engine = match engine {
                    ContainerEngine::Docker => ContainerEngine::Podman,
                    ContainerEngine::Podman => ContainerEngine::Docker,
                };

                if let Some(path) = default_socket_path(other_engine) {
                    eprintln!(
                        "Warning: {} (configured) is unavailable, falling back to {}",
                        engine, other_engine
                    );
                    used_fallback = true;
                    // Update last_used but NOT name (keep user's config)
                    config_file.engine.last_used = Some(other_engine.to_string());
                    self.save_config_file(&config_file)?;
                    path
                } else {
                    return Err(TskConfigError::NoContainerEngine);
                }
            }
        };

        // Update last_used if not a fallback
        if !used_fallback && config_file.engine.last_used.as_deref() != Some(&engine.to_string()) {
            config_file.engine.last_used = Some(engine.to_string());
            self.save_config_file(&config_file)?;
        }

        Ok((EngineConfig::new(engine, socket_path), used_fallback))
    }

    fn resolve_data_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf, TskConfigError> {
        // Check override first
        if let Some(data_dir) = override_dir {
            return Ok(data_dir.join("tsk"));
        }

        // Check XDG_DATA_HOME environment variable
        if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
            return Ok(PathBuf::from(xdg_data).join("tsk"));
        }

        // Fall back to ~/.local/share/tsk
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| TskConfigError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".local").join("share").join("tsk"))
    }

    fn resolve_runtime_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf, TskConfigError> {
        // Check override first
        if let Some(runtime_dir) = override_dir {
            return Ok(runtime_dir.join("tsk"));
        }

        // Check XDG_RUNTIME_DIR environment variable
        if let Ok(xdg_runtime) = env::var("XDG_RUNTIME_DIR") {
            return Ok(PathBuf::from(xdg_runtime).join("tsk"));
        }

        // Fall back to /tmp/tsk-$UID
        let uid = env::var("UID").unwrap_or_else(|_| {
            // On systems without UID env var, use current user ID
            #[cfg(unix)]
            {
                unsafe { libc::getuid().to_string() }
            }
            #[cfg(not(unix))]
            {
                "0".to_string()
            }
        });

        Ok(PathBuf::from("/tmp").join(format!("tsk-{uid}")))
    }

    fn resolve_config_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf, TskConfigError> {
        // Check override first
        if let Some(config_dir) = override_dir {
            return Ok(config_dir.join("tsk"));
        }

        // Check XDG_CONFIG_HOME environment variable
        if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(xdg_config).join("tsk"));
        }

        // Fall back to ~/.config/tsk
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| TskConfigError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".config").join("tsk"))
    }

    fn resolve_claude_config_dir(
        override_dir: Option<&PathBuf>,
    ) -> Result<PathBuf, TskConfigError> {
        // Check override first
        if let Some(claude_config_dir) = override_dir {
            return Ok(claude_config_dir.clone());
        }

        // Fall back to ~/.claude
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| TskConfigError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".claude"))
    }

    fn resolve_codex_config_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf, TskConfigError> {
        // Check override first
        if let Some(codex_config_dir) = override_dir {
            return Ok(codex_config_dir.clone());
        }

        // Fall back to ~/.codex
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| TskConfigError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".codex"))
    }

    fn resolve_editor(override_editor: Option<&String>) -> String {
        // Check override first
        if let Some(editor) = override_editor {
            return editor.clone();
        }

        // Check EDITOR environment variable, fall back to "vi"
        env::var("EDITOR").unwrap_or_else(|_| "vi".to_string())
    }

    fn resolve_terminal_type(override_terminal: Option<&Option<String>>) -> Option<String> {
        // Check override first
        if let Some(terminal_type) = override_terminal {
            return terminal_type.clone();
        }

        // Check TERM environment variable
        env::var("TERM").ok()
    }

    fn resolve_git_user_name(override_name: Option<&String>) -> Result<String, TskConfigError> {
        // Check override first
        if let Some(name) = override_name {
            return Ok(name.clone());
        }

        // Fall back to git config
        get_git_config("user.name")
    }

    fn resolve_git_user_email(override_email: Option<&String>) -> Result<String, TskConfigError> {
        // Check override first
        if let Some(email) = override_email {
            return Ok(email.clone());
        }

        // Fall back to git config
        get_git_config("user.email")
    }
}

impl Default for TskConfig {
    /// Create a TskConfig with default settings from environment
    fn default() -> Self {
        Self::new().expect("Failed to create default TskConfig")
    }
}

/// Builder for creating TskConfig instances with custom values
#[cfg(test)]
pub struct TskConfigBuilder {
    data_dir: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    claude_config_dir: Option<PathBuf>,
    codex_config_dir: Option<PathBuf>,
    editor: Option<String>,
    terminal_type: Option<Option<String>>,
    git_user_name: Option<String>,
    git_user_email: Option<String>,
}

#[cfg(test)]
impl TskConfigBuilder {
    /// Creates a new TskConfigBuilder
    pub fn new() -> Self {
        Self {
            data_dir: None,
            runtime_dir: None,
            config_dir: None,
            claude_config_dir: None,
            codex_config_dir: None,
            editor: None,
            terminal_type: None,
            git_user_name: None,
            git_user_email: None,
        }
    }

    /// Sets the data directory
    pub fn with_data_dir(mut self, dir: PathBuf) -> Self {
        self.data_dir = Some(dir);
        self
    }

    /// Sets the runtime directory
    pub fn with_runtime_dir(mut self, dir: PathBuf) -> Self {
        self.runtime_dir = Some(dir);
        self
    }

    /// Sets the config directory
    pub fn with_config_dir(mut self, dir: PathBuf) -> Self {
        self.config_dir = Some(dir);
        self
    }

    /// Sets the Claude configuration directory
    pub fn with_claude_config_dir(mut self, dir: PathBuf) -> Self {
        self.claude_config_dir = Some(dir);
        self
    }

    /// Sets the Codex configuration directory
    pub fn with_codex_config_dir(mut self, dir: PathBuf) -> Self {
        self.codex_config_dir = Some(dir);
        self
    }

    /// Sets the editor command
    pub fn with_editor(mut self, editor: String) -> Self {
        self.editor = Some(editor);
        self
    }

    /// Sets the terminal type
    pub fn with_terminal_type(mut self, terminal_type: Option<String>) -> Self {
        self.terminal_type = Some(terminal_type);
        self
    }

    /// Sets the git user name
    pub fn with_git_user_name(mut self, name: String) -> Self {
        self.git_user_name = Some(name);
        self
    }

    /// Sets the git user email
    pub fn with_git_user_email(mut self, email: String) -> Self {
        self.git_user_email = Some(email);
        self
    }

    /// Builds the TskConfig instance
    pub fn build(self) -> Result<TskConfig, TskConfigError> {
        let data_dir = TskConfig::resolve_data_dir(self.data_dir.as_ref())?;
        let runtime_dir = TskConfig::resolve_runtime_dir(self.runtime_dir.as_ref())?;
        let config_dir = TskConfig::resolve_config_dir(self.config_dir.as_ref())?;
        let claude_config_dir =
            TskConfig::resolve_claude_config_dir(self.claude_config_dir.as_ref())?;
        let codex_config_dir = TskConfig::resolve_codex_config_dir(self.codex_config_dir.as_ref())?;
        let editor = TskConfig::resolve_editor(self.editor.as_ref());
        let terminal_type = TskConfig::resolve_terminal_type(self.terminal_type.as_ref());
        let git_user_name = TskConfig::resolve_git_user_name(self.git_user_name.as_ref())?;
        let git_user_email = TskConfig::resolve_git_user_email(self.git_user_email.as_ref())?;

        Ok(TskConfig {
            data_dir,
            runtime_dir,
            config_dir,
            claude_config_dir,
            codex_config_dir,
            editor,
            terminal_type,
            git_user_name,
            git_user_email,
        })
    }
}

#[cfg(test)]
impl Default for TskConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Get git configuration value
fn get_git_config(key: &str) -> Result<String, TskConfigError> {
    #[cfg(test)]
    {
        Err(TskConfigError::GitConfig(format!(
            "Git config '{}' should not be accessed directly in tests. \
             Use AppContext::builder().build() to create a correct test context with tsk_config. \
             The test AppContext automatically sets git user.name and user.email.",
            key
        )))
    }

    #[cfg(not(test))]
    {
        let output = Command::new("git")
            .args(["config", "--global", key])
            .output()
            .map_err(|e| {
                TskConfigError::GitConfig(format!("Failed to execute git config: {}", e))
            })?;

        if !output.status.success() {
            return Err(TskConfigError::GitConfig(format!(
                "Git config '{}' not set. Please configure git with your name and email:\n\
                 git config --global user.name \"Your Name\"\n\
                 git config --global user.email \"your.email@example.com\"",
                key
            )));
        }

        let value = String::from_utf8(output.stdout)
            .map_err(|_| {
                TskConfigError::GitConfig("Git config output is not valid UTF-8".to_string())
            })?
            .trim()
            .to_string();

        if value.is_empty() {
            return Err(TskConfigError::GitConfig(format!(
                "Git config '{}' is empty. Please configure git with your name and email.",
                key
            )));
        }

        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;

    #[test]
    fn test_tsk_config_with_builder() {
        let config = TskConfig::builder()
            .with_data_dir(PathBuf::from("/custom/data"))
            .with_runtime_dir(PathBuf::from("/custom/runtime"))
            .with_config_dir(PathBuf::from("/custom/config"))
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build()
            .expect("Failed to create TSK configuration");

        assert_eq!(config.data_dir(), Path::new("/custom/data/tsk"));
        assert_eq!(config.runtime_dir(), Path::new("/custom/runtime/tsk"));
        assert_eq!(config.config_dir(), Path::new("/custom/config/tsk"));
        assert_eq!(
            config.tasks_file(),
            Path::new("/custom/data/tsk/tasks.json")
        );
        assert_eq!(
            config.socket_path(),
            Path::new("/custom/runtime/tsk/tsk.sock")
        );
        assert_eq!(config.pid_file(), Path::new("/custom/runtime/tsk/tsk.pid"));
        // Check that environment fields have defaults
        assert!(!config.editor().is_empty());
        assert!(
            config
                .claude_config_dir()
                .to_string_lossy()
                .contains(".claude")
        );
    }

    #[test]
    fn test_tsk_config_fallback() {
        // Test that fallback paths work when no overrides are provided
        // In tests, we need to provide git config
        let config = TskConfig::builder()
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build()
            .expect("Failed to create TSK configuration");

        // These paths depend on environment variables, so we just verify they contain "tsk"
        assert!(config.data_dir().to_string_lossy().contains("tsk"));
        assert!(config.runtime_dir().to_string_lossy().contains("tsk"));
        assert!(config.config_dir().to_string_lossy().contains("tsk"));
    }

    #[test]
    fn test_partial_config_overrides() {
        // Test that partial configs work correctly - only override some paths
        let config = TskConfig::builder()
            .with_data_dir(PathBuf::from("/override/data"))
            .with_config_dir(PathBuf::from("/override/config"))
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build()
            .expect("Failed to create TSK configuration");

        assert_eq!(config.data_dir(), Path::new("/override/data/tsk"));
        assert_eq!(config.config_dir(), Path::new("/override/config/tsk"));
        // Runtime dir should use environment variable or default
        assert!(config.runtime_dir().to_string_lossy().contains("tsk"));
    }

    #[test]
    fn test_config_resolution_priority() {
        // Test that config overrides work without environment manipulation
        let config = TskConfig::builder()
            .with_data_dir(PathBuf::from("/config/data"))
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build()
            .expect("Failed to create TSK configuration");

        // Config should be used as provided
        assert_eq!(config.data_dir(), Path::new("/config/data/tsk"));

        // Runtime and config dirs will use defaults or environment
        // Just verify they are set to something
        assert!(!config.runtime_dir().as_os_str().is_empty());
        assert!(!config.config_dir().as_os_str().is_empty());
    }

    #[test]
    fn test_task_dir_generation() {
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        let task_dir = config.task_dir("task-123", "repo-abc");

        assert!(task_dir.to_string_lossy().contains("repo-abc-task-123"));
    }

    #[test]
    fn test_tsk_config_builder_with_environment_fields() {
        let config = TskConfig::builder()
            .with_claude_config_dir(PathBuf::from("/test/.claude"))
            .with_editor("emacs".to_string())
            .with_terminal_type(Some("xterm-256color".to_string()))
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build()
            .expect("Failed to create TSK configuration");

        assert_eq!(config.claude_config_dir(), Path::new("/test/.claude"));
        assert_eq!(config.editor(), "emacs");
        assert_eq!(config.terminal_type(), Some("xterm-256color"));
    }

    #[test]
    fn test_tsk_config_builder_partial_environment() {
        let config = TskConfig::builder()
            .with_editor("nano".to_string())
            .with_terminal_type(None)
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build()
            .expect("Failed to create TSK configuration");

        assert_eq!(config.editor(), "nano");
        assert_eq!(config.terminal_type(), None);
        // Claude config dir should use default
        assert!(
            config
                .claude_config_dir()
                .to_string_lossy()
                .contains(".claude")
        );
    }

    #[test]
    fn test_tsk_config_builder_with_codex_config_dir() {
        let config = TskConfig::builder()
            .with_codex_config_dir(PathBuf::from("/test/.codex"))
            .with_git_user_name("Test User".to_string())
            .with_git_user_email("test@example.com".to_string())
            .build()
            .expect("Failed to create TSK configuration");

        assert_eq!(config.codex_config_dir(), Path::new("/test/.codex"));
    }

    #[test]
    fn test_git_configuration_overrides() {
        let config = TskConfig::builder()
            .with_git_user_name("Override User".to_string())
            .with_git_user_email("override@example.com".to_string())
            .build()
            .expect("Failed to create TSK configuration");

        assert_eq!(config.git_user_name(), "Override User");
        assert_eq!(config.git_user_email(), "override@example.com");
    }

    #[test]
    fn test_git_configuration_partial_override() {
        // Test that we can override just the name and git email will fall back to git config
        let builder = TskConfig::builder().with_git_user_name("Partial Override".to_string());

        // This test may fail if git is not configured on the system
        match builder.build() {
            Ok(config) => {
                assert_eq!(config.git_user_name(), "Partial Override");
                // git_user_email will be from git config or will cause error
                assert!(!config.git_user_email().is_empty());
            }
            Err(e) => {
                // Expected if git user.email is not configured
                assert!(e.to_string().contains("Git config"));
            }
        }
    }

    #[test]
    fn test_default_implementation() {
        // Default should work if git is configured, or fail gracefully
        let result = std::panic::catch_unwind(TskConfig::default);
        if let Ok(config) = result {
            assert!(!config.editor().is_empty());
            assert!(config.data_dir().to_string_lossy().contains("tsk"));
        }
    }

    #[test]
    fn test_config_file_path() {
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        assert!(config.config_file().to_string_lossy().contains("config.toml"));
    }

    #[test]
    fn test_load_missing_config_file() {
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();
        let result = config.load_config_file();
        assert!(result.is_ok());
        let config_file = result.unwrap();
        assert!(config_file.engine.name.is_none());
    }

    #[test]
    fn test_save_and_load_config_file() {
        let ctx = AppContext::builder().build();
        let config = ctx.tsk_config();

        let mut config_file = ConfigFile::default();
        config_file.engine.name = Some("podman".to_string());
        config_file.engine.last_used = Some("podman".to_string());

        config.save_config_file(&config_file).unwrap();

        let loaded = config.load_config_file().unwrap();
        assert_eq!(loaded.engine.name, Some("podman".to_string()));
        assert_eq!(loaded.engine.last_used, Some("podman".to_string()));
    }
}
