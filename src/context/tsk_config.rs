use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TskConfigError {
    #[error("Failed to determine home directory")]
    NoHomeDirectory,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Git configuration error: {0}")]
    GitConfig(String),
}

/// Configuration for overriding TSK configuration paths and environment settings
#[derive(Debug, Clone, Default)]
pub struct XdgConfig {
    /// Override for data directory (XDG_DATA_HOME)
    pub data_dir: Option<PathBuf>,
    /// Override for runtime directory (XDG_RUNTIME_DIR)
    pub runtime_dir: Option<PathBuf>,
    /// Override for config directory (XDG_CONFIG_HOME)
    pub config_dir: Option<PathBuf>,
    /// Override for Claude configuration directory (defaults to $HOME/.claude)
    pub claude_config_dir: Option<PathBuf>,
    /// Override for editor command (defaults to $EDITOR or "vi")
    pub editor: Option<String>,
    /// Override for terminal type (from $TERM)
    pub terminal_type: Option<Option<String>>,
    /// Override for git user.name (defaults to git config --global user.name)
    pub git_user_name: Option<String>,
    /// Override for git user.email (defaults to git config --global user.email)
    pub git_user_email: Option<String>,
}

impl XdgConfig {
    /// Create a new XdgConfig with all overrides set
    ///
    /// This method is used extensively throughout the codebase for testing with temporary directories
    #[allow(dead_code)] // Used throughout codebase for testing
    pub fn with_paths(data_dir: PathBuf, runtime_dir: PathBuf, config_dir: PathBuf) -> Self {
        Self {
            data_dir: Some(data_dir),
            runtime_dir: Some(runtime_dir),
            config_dir: Some(config_dir),
            claude_config_dir: None,
            editor: None,
            terminal_type: None,
            git_user_name: None,
            git_user_email: None,
        }
    }

    /// Create a builder for XdgConfig with more fine-grained control
    #[allow(dead_code)] // Used in tests for creating custom configs
    pub fn builder() -> XdgConfigBuilder {
        XdgConfigBuilder::new()
    }
}

/// Builder for creating XdgConfig instances with custom values
#[allow(dead_code)] // Used in tests for creating custom configs
pub struct XdgConfigBuilder {
    data_dir: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    claude_config_dir: Option<PathBuf>,
    editor: Option<String>,
    terminal_type: Option<Option<String>>,
    git_user_name: Option<String>,
    git_user_email: Option<String>,
}

#[allow(dead_code)] // Used in tests for creating custom configs  
impl XdgConfigBuilder {
    /// Creates a new XdgConfigBuilder
    pub fn new() -> Self {
        Self {
            data_dir: None,
            runtime_dir: None,
            config_dir: None,
            claude_config_dir: None,
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

    /// Builds the XdgConfig instance
    pub fn build(self) -> XdgConfig {
        XdgConfig {
            data_dir: self.data_dir,
            runtime_dir: self.runtime_dir,
            config_dir: self.config_dir,
            claude_config_dir: self.claude_config_dir,
            editor: self.editor,
            terminal_type: self.terminal_type,
            git_user_name: self.git_user_name,
            git_user_email: self.git_user_email,
        }
    }
}

impl Default for XdgConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Provides access to XDG Base Directory compliant paths for TSK
#[derive(Debug, Clone)]
pub struct TskConfig {
    data_dir: PathBuf,
    runtime_dir: PathBuf,
    config_dir: PathBuf,
    claude_config_dir: PathBuf,
    editor: String,
    terminal_type: Option<String>,
    git_user_name: String,
    git_user_email: String,
}

impl TskConfig {
    /// Create new TSK configuration instance with optional configuration overrides
    ///
    /// # Arguments
    /// * `config` - Optional configuration to override default XDG paths. If `None`,
    ///   environment variables and defaults will be used.
    pub fn new(config: Option<XdgConfig>) -> Result<Self, TskConfigError> {
        let config = config.unwrap_or_default();
        let data_dir = Self::resolve_data_dir(&config)?;
        let runtime_dir = Self::resolve_runtime_dir(&config)?;
        let config_dir = Self::resolve_config_dir(&config)?;
        let claude_config_dir = Self::resolve_claude_config_dir(&config)?;
        let editor = Self::resolve_editor(&config);
        let terminal_type = Self::resolve_terminal_type(&config);
        let git_user_name = Self::resolve_git_user_name(&config)?;
        let git_user_email = Self::resolve_git_user_email(&config)?;

        Ok(Self {
            data_dir,
            runtime_dir,
            config_dir,
            claude_config_dir,
            editor,
            terminal_type,
            git_user_name,
            git_user_email,
        })
    }

    /// Get the data directory path (for persistent storage)
    ///
    /// Used by task.rs for accessing task storage directory
    #[allow(dead_code)] // Used by task.rs
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

    fn resolve_data_dir(config: &XdgConfig) -> Result<PathBuf, TskConfigError> {
        // Check config override first
        if let Some(ref data_dir) = config.data_dir {
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

    fn resolve_runtime_dir(config: &XdgConfig) -> Result<PathBuf, TskConfigError> {
        // Check config override first
        if let Some(ref runtime_dir) = config.runtime_dir {
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

    fn resolve_config_dir(config: &XdgConfig) -> Result<PathBuf, TskConfigError> {
        // Check config override first
        if let Some(ref config_dir) = config.config_dir {
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

    fn resolve_claude_config_dir(config: &XdgConfig) -> Result<PathBuf, TskConfigError> {
        // Check config override first
        if let Some(ref claude_config_dir) = config.claude_config_dir {
            return Ok(claude_config_dir.clone());
        }

        // Fall back to ~/.claude
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| TskConfigError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".claude"))
    }

    fn resolve_editor(config: &XdgConfig) -> String {
        // Check config override first
        if let Some(ref editor) = config.editor {
            return editor.clone();
        }

        // Check EDITOR environment variable, fall back to "vi"
        env::var("EDITOR").unwrap_or_else(|_| "vi".to_string())
    }

    fn resolve_terminal_type(config: &XdgConfig) -> Option<String> {
        // Check config override first
        if let Some(ref terminal_type) = config.terminal_type {
            return terminal_type.clone();
        }

        // Check TERM environment variable
        env::var("TERM").ok()
    }

    fn resolve_git_user_name(config: &XdgConfig) -> Result<String, TskConfigError> {
        // Check config override first
        if let Some(ref name) = config.git_user_name {
            return Ok(name.clone());
        }

        // Fall back to git config
        get_git_config("user.name")
    }

    fn resolve_git_user_email(config: &XdgConfig) -> Result<String, TskConfigError> {
        // Check config override first
        if let Some(ref email) = config.git_user_email {
            return Ok(email.clone());
        }

        // Fall back to git config
        get_git_config("user.email")
    }
}

/// Get git configuration value
fn get_git_config(key: &str) -> Result<String, TskConfigError> {
    let output = Command::new("git")
        .args(["config", "--global", key])
        .output()
        .map_err(|e| TskConfigError::GitConfig(format!("Failed to execute git config: {}", e)))?;

    if !output.status.success() {
        return Err(TskConfigError::GitConfig(format!(
            "Git config '{}' not set. Please configure git with your name and email:\n\
             git config --global user.name \"Your Name\"\n\
             git config --global user.email \"your.email@example.com\"",
            key
        )));
    }

    let value = String::from_utf8(output.stdout)
        .map_err(|_| TskConfigError::GitConfig("Git config output is not valid UTF-8".to_string()))?
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tsk_config_with_config() {
        let config = XdgConfig::with_paths(
            PathBuf::from("/custom/data"),
            PathBuf::from("/custom/runtime"),
            PathBuf::from("/custom/config"),
        );

        let dirs = TskConfig::new(Some(config)).expect("Failed to create TSK configuration");

        assert_eq!(dirs.data_dir(), Path::new("/custom/data/tsk"));
        assert_eq!(dirs.runtime_dir(), Path::new("/custom/runtime/tsk"));
        assert_eq!(dirs.config_dir(), Path::new("/custom/config/tsk"));
        assert_eq!(dirs.tasks_file(), Path::new("/custom/data/tsk/tasks.json"));
        assert_eq!(
            dirs.socket_path(),
            Path::new("/custom/runtime/tsk/tsk.sock")
        );
        assert_eq!(dirs.pid_file(), Path::new("/custom/runtime/tsk/tsk.pid"));
        // Check that environment fields have defaults
        assert!(!dirs.editor().is_empty());
        assert!(
            dirs.claude_config_dir()
                .to_string_lossy()
                .contains(".claude")
        );
    }

    #[test]
    fn test_tsk_config_fallback() {
        // Test that fallback paths work when no config is provided
        let dirs = TskConfig::new(None).expect("Failed to create TSK configuration");

        // These paths depend on environment variables, so we just verify they contain "tsk"
        assert!(dirs.data_dir().to_string_lossy().contains("tsk"));
        assert!(dirs.runtime_dir().to_string_lossy().contains("tsk"));
        assert!(dirs.config_dir().to_string_lossy().contains("tsk"));
    }

    #[test]
    fn test_partial_config_overrides() {
        // Test that partial configs work correctly - only override some paths
        let config = XdgConfig {
            data_dir: Some(PathBuf::from("/override/data")),
            runtime_dir: None,
            config_dir: Some(PathBuf::from("/override/config")),
            claude_config_dir: None,
            editor: None,
            terminal_type: None,
            git_user_name: None,
            git_user_email: None,
        };

        let dirs = TskConfig::new(Some(config)).expect("Failed to create TSK configuration");

        assert_eq!(dirs.data_dir(), Path::new("/override/data/tsk"));
        assert_eq!(dirs.config_dir(), Path::new("/override/config/tsk"));
        // Runtime dir should use environment variable or default
        assert!(dirs.runtime_dir().to_string_lossy().contains("tsk"));
    }

    #[test]
    fn test_config_resolution_priority() {
        // Test that config overrides work without environment manipulation
        // The XdgConfig mechanism provides a safe way to override paths
        let config = XdgConfig {
            data_dir: Some(PathBuf::from("/config/data")),
            runtime_dir: None,
            config_dir: None,
            claude_config_dir: None,
            editor: None,
            terminal_type: None,
            git_user_name: None,
            git_user_email: None,
        };

        let dirs = TskConfig::new(Some(config)).expect("Failed to create TSK configuration");

        // Config should be used as provided
        assert_eq!(dirs.data_dir(), Path::new("/config/data/tsk"));

        // Runtime and config dirs will use defaults or environment
        // Just verify they are set to something
        assert!(!dirs.runtime_dir().as_os_str().is_empty());
        assert!(!dirs.config_dir().as_os_str().is_empty());
    }

    #[test]
    fn test_task_dir_generation() {
        let dirs = TskConfig::new(None).expect("Failed to create TSK configuration");
        let task_dir = dirs.task_dir("task-123", "repo-abc");

        assert!(task_dir.to_string_lossy().contains("repo-abc-task-123"));
    }

    #[test]
    fn test_xdg_config_builder_with_environment_fields() {
        let config = XdgConfig::builder()
            .with_claude_config_dir(PathBuf::from("/test/.claude"))
            .with_editor("emacs".to_string())
            .with_terminal_type(Some("xterm-256color".to_string()))
            .build();

        let dirs = TskConfig::new(Some(config)).expect("Failed to create TSK configuration");

        assert_eq!(dirs.claude_config_dir(), Path::new("/test/.claude"));
        assert_eq!(dirs.editor(), "emacs");
        assert_eq!(dirs.terminal_type(), Some("xterm-256color"));
    }

    #[test]
    fn test_xdg_config_builder_partial_environment() {
        let config = XdgConfig::builder()
            .with_editor("nano".to_string())
            .with_terminal_type(None)
            .build();

        let dirs = TskConfig::new(Some(config)).expect("Failed to create TSK configuration");

        assert_eq!(dirs.editor(), "nano");
        assert_eq!(dirs.terminal_type(), None);
        // Claude config dir should use default
        assert!(
            dirs.claude_config_dir()
                .to_string_lossy()
                .contains(".claude")
        );
    }

    #[test]
    fn test_git_configuration_overrides() {
        let config = XdgConfig::builder()
            .with_git_user_name("Override User".to_string())
            .with_git_user_email("override@example.com".to_string())
            .build();

        let dirs = TskConfig::new(Some(config)).expect("Failed to create TSK configuration");

        assert_eq!(dirs.git_user_name(), "Override User");
        assert_eq!(dirs.git_user_email(), "override@example.com");
    }

    #[test]
    fn test_git_configuration_partial_override() {
        // Test that we can override just the name and git email will fall back to git config
        let config = XdgConfig::builder()
            .with_git_user_name("Partial Override".to_string())
            .build();

        // This test may fail if git is not configured on the system
        match TskConfig::new(Some(config)) {
            Ok(dirs) => {
                assert_eq!(dirs.git_user_name(), "Partial Override");
                // git_user_email will be from git config or will cause error
                assert!(!dirs.git_user_email().is_empty());
            }
            Err(e) => {
                // Expected if git user.email is not configured
                assert!(e.to_string().contains("Git config"));
            }
        }
    }
}
