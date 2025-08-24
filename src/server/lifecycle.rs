use crate::context::tsk_config::TskConfig;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process;
use std::sync::Arc;

/// Manages server lifecycle (PID files, etc.)
pub struct ServerLifecycle {
    tsk_config: Arc<TskConfig>,
}

impl ServerLifecycle {
    /// Create a new server lifecycle manager
    pub fn new(tsk_config: Arc<TskConfig>) -> Self {
        Self { tsk_config }
    }

    /// Check if a server is already running
    pub fn is_server_running(&self) -> bool {
        let pid_file = self.tsk_config.pid_file();

        if !pid_file.exists() {
            return false;
        }

        // Read PID from file
        match self.read_pid() {
            Some(pid) => {
                // Check if process is still alive
                self.is_process_alive(pid)
            }
            None => false,
        }
    }

    /// Write current process PID to file
    pub fn write_pid(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let pid_file = self.tsk_config.pid_file();
        let pid = process::id();

        let mut file = fs::File::create(&pid_file)?;
        file.write_all(pid.to_string().as_bytes())?;

        Ok(())
    }

    /// Read PID from file
    pub fn read_pid(&self) -> Option<u32> {
        let pid_file = self.tsk_config.pid_file();

        let mut file = match fs::File::open(&pid_file) {
            Ok(f) => f,
            Err(_) => return None,
        };

        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_err() {
            return None;
        }

        contents.trim().parse().ok()
    }

    /// Remove PID file
    pub fn remove_pid(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let pid_file = self.tsk_config.pid_file();
        if pid_file.exists() {
            fs::remove_file(&pid_file)?;
        }
        Ok(())
    }

    /// Check if a process with given PID is alive
    #[cfg(unix)]
    fn is_process_alive(&self, pid: u32) -> bool {
        unsafe {
            // Send signal 0 to check if process exists
            libc::kill(pid as i32, 0) == 0
        }
    }

    #[cfg(not(unix))]
    fn is_process_alive(&self, _pid: u32) -> bool {
        // On non-Unix systems, assume the process is alive if PID file exists
        true
    }

    /// Get the server socket path
    pub fn socket_path(&self) -> PathBuf {
        self.tsk_config.socket_path()
    }

    /// Clean up server resources
    pub fn cleanup(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Remove PID file
        self.remove_pid()?;

        // Remove socket file
        let socket_path = self.socket_path();
        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;

    #[test]
    fn test_server_lifecycle() {
        let app_context = AppContext::builder().build();
        let config = app_context.tsk_config();

        let lifecycle = ServerLifecycle::new(config);

        // Initially no server should be running
        assert!(!lifecycle.is_server_running());

        // Write PID
        lifecycle.write_pid().unwrap();

        // Now server should be detected as running
        assert!(lifecycle.is_server_running());

        // Read PID should return current process ID
        let pid = lifecycle.read_pid().unwrap();
        assert_eq!(pid, process::id());

        // Cleanup should remove PID file
        lifecycle.cleanup().unwrap();
        assert!(!lifecycle.is_server_running());
    }
}
