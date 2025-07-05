use std::env;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum XdgError {
    #[error("Failed to determine home directory")]
    NoHomeDirectory,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Configuration for overriding XDG directory paths
#[derive(Debug, Clone, Default)]
pub struct XdgConfig {
    /// Override for data directory (XDG_DATA_HOME)
    pub data_dir: Option<PathBuf>,
    /// Override for runtime directory (XDG_RUNTIME_DIR)
    pub runtime_dir: Option<PathBuf>,
    /// Override for config directory (XDG_CONFIG_HOME)
    pub config_dir: Option<PathBuf>,
}

impl XdgConfig {
    /// Create a new XdgConfig with all overrides set
    pub fn with_paths(data_dir: PathBuf, runtime_dir: PathBuf, config_dir: PathBuf) -> Self {
        Self {
            data_dir: Some(data_dir),
            runtime_dir: Some(runtime_dir),
            config_dir: Some(config_dir),
        }
    }
}

/// Provides access to XDG Base Directory compliant paths for TSK
#[derive(Debug, Clone)]
pub struct XdgDirectories {
    data_dir: PathBuf,
    runtime_dir: PathBuf,
    config_dir: PathBuf,
}

impl XdgDirectories {
    /// Create new XDG directories instance with optional configuration overrides
    ///
    /// # Arguments
    /// * `config` - Optional configuration to override default XDG paths. If `None`,
    ///   environment variables and defaults will be used.
    pub fn new(config: Option<XdgConfig>) -> Result<Self, XdgError> {
        let config = config.unwrap_or_default();
        let data_dir = Self::resolve_data_dir(&config)?;
        let runtime_dir = Self::resolve_runtime_dir(&config)?;
        let config_dir = Self::resolve_config_dir(&config)?;

        Ok(Self {
            data_dir,
            runtime_dir,
            config_dir,
        })
    }

    /// Get the data directory path (for persistent storage)
    #[allow(dead_code)]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Get the runtime directory path (for sockets, pid files)
    #[allow(dead_code)]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// Get the config directory path (for configuration files)
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Get the templates directory path
    #[allow(dead_code)]
    pub fn templates_dir(&self) -> PathBuf {
        self.config_dir.join("templates")
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
    pub fn ensure_directories(&self) -> Result<(), XdgError> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(self.data_dir.join("tasks"))?;
        std::fs::create_dir_all(&self.runtime_dir)?;
        std::fs::create_dir_all(&self.config_dir)?;
        Ok(())
    }

    fn resolve_data_dir(config: &XdgConfig) -> Result<PathBuf, XdgError> {
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
            .map_err(|_| XdgError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".local").join("share").join("tsk"))
    }

    fn resolve_runtime_dir(config: &XdgConfig) -> Result<PathBuf, XdgError> {
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

    fn resolve_config_dir(config: &XdgConfig) -> Result<PathBuf, XdgError> {
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
            .map_err(|_| XdgError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".config").join("tsk"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xdg_directories_with_config() {
        let config = XdgConfig::with_paths(
            PathBuf::from("/custom/data"),
            PathBuf::from("/custom/runtime"),
            PathBuf::from("/custom/config"),
        );

        let dirs = XdgDirectories::new(Some(config)).expect("Failed to create XDG directories");

        assert_eq!(dirs.data_dir(), Path::new("/custom/data/tsk"));
        assert_eq!(dirs.runtime_dir(), Path::new("/custom/runtime/tsk"));
        assert_eq!(dirs.config_dir(), Path::new("/custom/config/tsk"));
        assert_eq!(dirs.tasks_file(), Path::new("/custom/data/tsk/tasks.json"));
        assert_eq!(
            dirs.socket_path(),
            Path::new("/custom/runtime/tsk/tsk.sock")
        );
        assert_eq!(dirs.pid_file(), Path::new("/custom/runtime/tsk/tsk.pid"));
        assert_eq!(
            dirs.templates_dir(),
            Path::new("/custom/config/tsk/templates")
        );
    }

    #[test]
    fn test_xdg_directories_fallback() {
        // Test that fallback paths work when no config is provided
        let dirs = XdgDirectories::new(None).expect("Failed to create XDG directories");

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
        };

        let dirs = XdgDirectories::new(Some(config)).expect("Failed to create XDG directories");

        assert_eq!(dirs.data_dir(), Path::new("/override/data/tsk"));
        assert_eq!(dirs.config_dir(), Path::new("/override/config/tsk"));
        // Runtime dir should use environment variable or default
        assert!(dirs.runtime_dir().to_string_lossy().contains("tsk"));
    }

    #[test]
    fn test_config_resolution_priority() {
        // Test that config overrides take precedence over environment variables
        let original_data = env::var("XDG_DATA_HOME").ok();

        // Set environment variable
        unsafe {
            env::set_var("XDG_DATA_HOME", "/env/data");
        }

        // But provide config override
        let config = XdgConfig {
            data_dir: Some(PathBuf::from("/config/data")),
            runtime_dir: None,
            config_dir: None,
        };

        let dirs = XdgDirectories::new(Some(config)).expect("Failed to create XDG directories");

        // Config should take precedence
        assert_eq!(dirs.data_dir(), Path::new("/config/data/tsk"));

        // Restore original environment
        match original_data {
            Some(val) => unsafe { env::set_var("XDG_DATA_HOME", val) },
            None => unsafe { env::remove_var("XDG_DATA_HOME") },
        }
    }

    #[test]
    fn test_task_dir_generation() {
        let dirs = XdgDirectories::new(None).expect("Failed to create XDG directories");
        let task_dir = dirs.task_dir("task-123", "repo-abc");

        assert!(task_dir.to_string_lossy().contains("repo-abc-task-123"));
    }
}
