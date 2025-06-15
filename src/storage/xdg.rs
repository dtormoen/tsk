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

/// Provides access to XDG Base Directory compliant paths for TSK
#[derive(Debug, Clone)]
pub struct XdgDirectories {
    data_dir: PathBuf,
    runtime_dir: PathBuf,
}

impl XdgDirectories {
    /// Create new XDG directories instance with standard paths
    pub fn new() -> Result<Self, XdgError> {
        let data_dir = Self::resolve_data_dir()?;
        let runtime_dir = Self::resolve_runtime_dir()?;

        Ok(Self {
            data_dir,
            runtime_dir,
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

    /// Get the path to the tasks.json file
    pub fn tasks_file(&self) -> PathBuf {
        self.data_dir.join("tasks.json")
    }

    /// Get the path to a task's directory
    pub fn task_dir(&self, task_id: &str, repo_hash: &str) -> PathBuf {
        self.data_dir
            .join("tasks")
            .join(format!("{}-{}", repo_hash, task_id))
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
        Ok(())
    }

    fn resolve_data_dir() -> Result<PathBuf, XdgError> {
        // Check XDG_DATA_HOME first
        if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
            return Ok(PathBuf::from(xdg_data).join("tsk"));
        }

        // Fall back to ~/.local/share/tsk
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .map_err(|_| XdgError::NoHomeDirectory)?;

        Ok(PathBuf::from(home).join(".local").join("share").join("tsk"))
    }

    fn resolve_runtime_dir() -> Result<PathBuf, XdgError> {
        // Check XDG_RUNTIME_DIR first
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

        Ok(PathBuf::from("/tmp").join(format!("tsk-{}", uid)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_xdg_directories_with_env_vars() {
        let original_data = env::var("XDG_DATA_HOME").ok();
        let original_runtime = env::var("XDG_RUNTIME_DIR").ok();

        env::set_var("XDG_DATA_HOME", "/custom/data");
        env::set_var("XDG_RUNTIME_DIR", "/custom/runtime");

        let dirs = XdgDirectories::new().expect("Failed to create XDG directories");

        assert_eq!(dirs.data_dir(), Path::new("/custom/data/tsk"));
        assert_eq!(dirs.runtime_dir(), Path::new("/custom/runtime/tsk"));
        assert_eq!(dirs.tasks_file(), Path::new("/custom/data/tsk/tasks.json"));
        assert_eq!(
            dirs.socket_path(),
            Path::new("/custom/runtime/tsk/tsk.sock")
        );
        assert_eq!(dirs.pid_file(), Path::new("/custom/runtime/tsk/tsk.pid"));

        // Restore original environment
        match original_data {
            Some(val) => env::set_var("XDG_DATA_HOME", val),
            None => env::remove_var("XDG_DATA_HOME"),
        }
        match original_runtime {
            Some(val) => env::set_var("XDG_RUNTIME_DIR", val),
            None => env::remove_var("XDG_RUNTIME_DIR"),
        }
    }

    #[test]
    fn test_xdg_directories_fallback() {
        let original_data = env::var("XDG_DATA_HOME").ok();
        let original_runtime = env::var("XDG_RUNTIME_DIR").ok();

        env::remove_var("XDG_DATA_HOME");
        env::remove_var("XDG_RUNTIME_DIR");

        let dirs = XdgDirectories::new().expect("Failed to create XDG directories");

        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .unwrap();
        let expected_data = PathBuf::from(&home)
            .join(".local")
            .join("share")
            .join("tsk");
        assert_eq!(dirs.data_dir(), expected_data);

        // Restore original environment
        match original_data {
            Some(val) => env::set_var("XDG_DATA_HOME", val),
            None => env::remove_var("XDG_DATA_HOME"),
        }
        match original_runtime {
            Some(val) => env::set_var("XDG_RUNTIME_DIR", val),
            None => env::remove_var("XDG_RUNTIME_DIR"),
        }
    }

    #[test]
    fn test_task_dir_generation() {
        let dirs = XdgDirectories::new().expect("Failed to create XDG directories");
        let task_dir = dirs.task_dir("task-123", "repo-abc");

        assert!(task_dir.to_string_lossy().contains("repo-abc-task-123"));
    }
}
