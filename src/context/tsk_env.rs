use std::env;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TskEnvError {
    #[error("Failed to determine home directory")]
    NoHomeDirectory,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Provides access to XDG Base Directory compliant paths for TSK
///
/// This struct contains runtime environment configuration including
/// XDG-compliant paths and editor settings.
/// It does NOT contain user-configurable options from tsk.toml -
/// those are managed separately.
///
/// Path resolution priority (highest to lowest):
/// 1. Builder override (test-only)
/// 2. TSK-specific env var (`TSK_DATA_HOME`, `TSK_RUNTIME_DIR`, `TSK_CONFIG_HOME`)
/// 3. XDG env var (`XDG_DATA_HOME`, `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`)
/// 4. Default fallback (`~/.local/share`, `/tmp`, `~/.config`)
///
/// TSK-specific env vars allow isolating TSK's directories without
/// affecting other XDG-aware software.
#[derive(Debug, Clone)]
pub struct TskEnv {
    data_dir: PathBuf,
    runtime_dir: PathBuf,
    config_dir: PathBuf,
    claude_config_dir: PathBuf,
    codex_config_dir: PathBuf,
    editor: String,
    terminal_type: Option<String>,
}

impl TskEnv {
    /// Create new TSK environment instance with default paths from environment
    ///
    /// Checks TSK-specific env vars first, then falls back to XDG:
    /// - TSK_DATA_HOME / XDG_DATA_HOME for data directory (defaults to ~/.local/share/tsk)
    /// - TSK_RUNTIME_DIR / XDG_RUNTIME_DIR for runtime directory (defaults to /tmp/tsk-$UID)
    /// - TSK_CONFIG_HOME / XDG_CONFIG_HOME for config directory (defaults to ~/.config/tsk)
    pub fn new() -> Result<Self, TskEnvError> {
        let data_dir = Self::resolve_data_dir(None)?;
        let runtime_dir = Self::resolve_runtime_dir(None)?;
        let config_dir = Self::resolve_config_dir(None)?;
        let claude_config_dir = Self::resolve_claude_config_dir(None)?;
        let codex_config_dir = Self::resolve_codex_config_dir(None)?;
        let editor = Self::resolve_editor(None);
        let terminal_type = Self::resolve_terminal_type(None);

        Ok(Self {
            data_dir,
            runtime_dir,
            config_dir,
            claude_config_dir,
            codex_config_dir,
            editor,
            terminal_type,
        })
    }

    /// Create a builder for TskEnv with custom overrides
    #[cfg(test)]
    pub fn builder() -> TskEnvBuilder {
        TskEnvBuilder::new()
    }

    /// Get the data directory path (for persistent storage)
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

    /// Get the legacy tasks.json path (used for migration; production storage is tasks.db)
    pub fn tasks_file(&self) -> PathBuf {
        self.data_dir.join("tasks.json")
    }

    /// Get the path to the tasks.db SQLite database
    pub fn tasks_db(&self) -> PathBuf {
        self.data_dir.join("tasks.db")
    }

    /// Get the path to a task's directory
    pub fn task_dir(&self, task_id: &str) -> PathBuf {
        self.data_dir.join("tasks").join(task_id)
    }

    /// Get the directory for per-proxy configuration files (e.g., squid.conf)
    pub fn proxy_config_dir(&self, fingerprint: &str) -> PathBuf {
        self.data_dir.join("proxy-configs").join(fingerprint)
    }

    /// Get the server PID file path
    pub fn pid_file(&self) -> PathBuf {
        self.runtime_dir.join("tsk.pid")
    }

    /// Ensure all required directories exist
    pub fn ensure_directories(&self) -> Result<(), TskEnvError> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(self.data_dir.join("tasks"))?;
        std::fs::create_dir_all(&self.runtime_dir)?;
        std::fs::create_dir_all(&self.config_dir)?;
        Ok(())
    }

    fn resolve_data_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf, TskEnvError> {
        // Check override first
        if let Some(data_dir) = override_dir {
            return Ok(data_dir.join("tsk"));
        }

        // Check TSK_DATA_HOME for TSK-specific isolation
        if let Ok(tsk_data) = env::var("TSK_DATA_HOME") {
            return Ok(PathBuf::from(tsk_data).join("tsk"));
        }

        // Check XDG_DATA_HOME environment variable
        if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
            return Ok(PathBuf::from(xdg_data).join("tsk"));
        }

        // Fall back to ~/.local/share/tsk
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| TskEnvError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".local").join("share").join("tsk"))
    }

    fn resolve_runtime_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf, TskEnvError> {
        // Check override first
        if let Some(runtime_dir) = override_dir {
            return Ok(runtime_dir.join("tsk"));
        }

        // Check TSK_RUNTIME_DIR for TSK-specific isolation
        if let Ok(tsk_runtime) = env::var("TSK_RUNTIME_DIR") {
            return Ok(PathBuf::from(tsk_runtime).join("tsk"));
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

    fn resolve_config_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf, TskEnvError> {
        // Check override first
        if let Some(config_dir) = override_dir {
            return Ok(config_dir.join("tsk"));
        }

        // Check TSK_CONFIG_HOME for TSK-specific isolation
        if let Ok(tsk_config) = env::var("TSK_CONFIG_HOME") {
            return Ok(PathBuf::from(tsk_config).join("tsk"));
        }

        // Check XDG_CONFIG_HOME environment variable
        if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(xdg_config).join("tsk"));
        }

        // Fall back to ~/.config/tsk
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| TskEnvError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".config").join("tsk"))
    }

    fn resolve_claude_config_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf, TskEnvError> {
        // Check override first
        if let Some(claude_config_dir) = override_dir {
            return Ok(claude_config_dir.clone());
        }

        // Fall back to ~/.claude
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| TskEnvError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".claude"))
    }

    fn resolve_codex_config_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf, TskEnvError> {
        // Check override first
        if let Some(codex_config_dir) = override_dir {
            return Ok(codex_config_dir.clone());
        }

        // Fall back to ~/.codex
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| TskEnvError::NoHomeDirectory)?;

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
}

impl Default for TskEnv {
    /// Create a TskEnv with default settings from environment
    fn default() -> Self {
        Self::new().expect("Failed to create default TskEnv")
    }
}

/// Builder for creating TskEnv instances with custom values
#[cfg(test)]
pub struct TskEnvBuilder {
    data_dir: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    claude_config_dir: Option<PathBuf>,
    codex_config_dir: Option<PathBuf>,
    editor: Option<String>,
    terminal_type: Option<Option<String>>,
}

#[cfg(test)]
impl TskEnvBuilder {
    /// Creates a new TskEnvBuilder
    pub fn new() -> Self {
        Self {
            data_dir: None,
            runtime_dir: None,
            config_dir: None,
            claude_config_dir: None,
            codex_config_dir: None,
            editor: None,
            terminal_type: None,
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

    /// Builds the TskEnv instance
    pub fn build(self) -> Result<TskEnv, TskEnvError> {
        let data_dir = TskEnv::resolve_data_dir(self.data_dir.as_ref())?;
        let runtime_dir = TskEnv::resolve_runtime_dir(self.runtime_dir.as_ref())?;
        let config_dir = TskEnv::resolve_config_dir(self.config_dir.as_ref())?;
        let claude_config_dir = TskEnv::resolve_claude_config_dir(self.claude_config_dir.as_ref())?;
        let codex_config_dir = TskEnv::resolve_codex_config_dir(self.codex_config_dir.as_ref())?;
        let editor = TskEnv::resolve_editor(self.editor.as_ref());
        let terminal_type = TskEnv::resolve_terminal_type(self.terminal_type.as_ref());

        Ok(TskEnv {
            data_dir,
            runtime_dir,
            config_dir,
            claude_config_dir,
            codex_config_dir,
            editor,
            terminal_type,
        })
    }
}

#[cfg(test)]
impl Default for TskEnvBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tsk_env_with_builder() {
        let env = TskEnv::builder()
            .with_data_dir(PathBuf::from("/custom/data"))
            .with_runtime_dir(PathBuf::from("/custom/runtime"))
            .with_config_dir(PathBuf::from("/custom/config"))
            .build()
            .expect("Failed to create TSK environment");

        assert_eq!(env.data_dir(), Path::new("/custom/data/tsk"));
        assert_eq!(env.runtime_dir(), Path::new("/custom/runtime/tsk"));
        assert_eq!(env.config_dir(), Path::new("/custom/config/tsk"));
        assert_eq!(env.tasks_file(), Path::new("/custom/data/tsk/tasks.json"));
        assert_eq!(env.tasks_db(), Path::new("/custom/data/tsk/tasks.db"));
        assert_eq!(env.pid_file(), Path::new("/custom/runtime/tsk/tsk.pid"));
        // Check that environment fields have defaults
        assert!(!env.editor().is_empty());
        assert!(
            env.claude_config_dir()
                .to_string_lossy()
                .contains(".claude")
        );
    }

    #[test]
    fn test_tsk_env_fallback() {
        // Test that fallback paths work when no overrides are provided
        let env = TskEnv::builder()
            .build()
            .expect("Failed to create TSK environment");

        // These paths depend on environment variables, so we just verify they contain "tsk"
        assert!(env.data_dir().to_string_lossy().contains("tsk"));
        assert!(env.runtime_dir().to_string_lossy().contains("tsk"));
        assert!(env.config_dir().to_string_lossy().contains("tsk"));
    }

    #[test]
    fn test_partial_env_overrides() {
        // Test that partial configs work correctly - only override some paths
        let env = TskEnv::builder()
            .with_data_dir(PathBuf::from("/override/data"))
            .with_config_dir(PathBuf::from("/override/config"))
            .build()
            .expect("Failed to create TSK environment");

        assert_eq!(env.data_dir(), Path::new("/override/data/tsk"));
        assert_eq!(env.config_dir(), Path::new("/override/config/tsk"));
        // Runtime dir should use environment variable or default
        assert!(env.runtime_dir().to_string_lossy().contains("tsk"));
    }

    #[test]
    fn test_env_resolution_priority() {
        // Test that env overrides work without environment manipulation
        let env = TskEnv::builder()
            .with_data_dir(PathBuf::from("/config/data"))
            .build()
            .expect("Failed to create TSK environment");

        // Env should be used as provided
        assert_eq!(env.data_dir(), Path::new("/config/data/tsk"));

        // Runtime and config dirs will use defaults or environment
        // Just verify they are set to something
        assert!(!env.runtime_dir().as_os_str().is_empty());
        assert!(!env.config_dir().as_os_str().is_empty());
    }

    #[test]
    fn test_task_dir_generation() {
        use crate::context::AppContext;

        let ctx = AppContext::builder().build();
        let tsk_env = ctx.tsk_env();
        let task_dir = tsk_env.task_dir("task-123");

        assert!(task_dir.ends_with("tasks/task-123"));
    }

    #[test]
    fn test_tsk_env_builder_with_environment_fields() {
        let env = TskEnv::builder()
            .with_claude_config_dir(PathBuf::from("/test/.claude"))
            .with_editor("emacs".to_string())
            .with_terminal_type(Some("xterm-256color".to_string()))
            .build()
            .expect("Failed to create TSK environment");

        assert_eq!(env.claude_config_dir(), Path::new("/test/.claude"));
        assert_eq!(env.editor(), "emacs");
        assert_eq!(env.terminal_type(), Some("xterm-256color"));
    }

    #[test]
    fn test_tsk_env_builder_partial_environment() {
        let env = TskEnv::builder()
            .with_editor("nano".to_string())
            .with_terminal_type(None)
            .build()
            .expect("Failed to create TSK environment");

        assert_eq!(env.editor(), "nano");
        assert_eq!(env.terminal_type(), None);
        // Claude config dir should use default
        assert!(
            env.claude_config_dir()
                .to_string_lossy()
                .contains(".claude")
        );
    }

    #[test]
    fn test_tsk_env_builder_with_codex_config_dir() {
        let env = TskEnv::builder()
            .with_codex_config_dir(PathBuf::from("/test/.codex"))
            .build()
            .expect("Failed to create TSK environment");

        assert_eq!(env.codex_config_dir(), Path::new("/test/.codex"));
    }

    #[test]
    fn test_default_implementation() {
        // Default should work based on environment settings
        let result = std::panic::catch_unwind(TskEnv::default);
        if let Ok(env) = result {
            assert!(!env.editor().is_empty());
            assert!(env.data_dir().to_string_lossy().contains("tsk"));
        }
    }
}
