use crate::context::tsk_env::TskEnv;
use std::fs;
use std::io::{Read, Write};
use std::process;
use std::sync::Arc;

/// Manages server lifecycle (PID files, signals, etc.)
pub struct ServerLifecycle {
    tsk_env: Arc<TskEnv>,
}

impl ServerLifecycle {
    /// Create a new server lifecycle manager
    pub fn new(tsk_env: Arc<TskEnv>) -> Self {
        Self { tsk_env }
    }

    /// Check if a server is already running
    pub fn is_server_running(&self) -> bool {
        let pid_file = self.tsk_env.pid_file();

        if !pid_file.exists() {
            return false;
        }

        match self.read_pid() {
            Some(pid) => self.is_process_alive(pid),
            None => false,
        }
    }

    /// Write current process PID to file
    pub fn write_pid(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let pid_file = self.tsk_env.pid_file();
        let pid = process::id();

        let mut file = fs::File::create(&pid_file)?;
        file.write_all(pid.to_string().as_bytes())?;

        Ok(())
    }

    /// Read PID from file
    pub fn read_pid(&self) -> Option<u32> {
        let pid_file = self.tsk_env.pid_file();

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
        let pid_file = self.tsk_env.pid_file();
        if pid_file.exists() {
            fs::remove_file(&pid_file)?;
        }
        Ok(())
    }

    /// Send SIGTERM to the server process
    #[cfg(unix)]
    pub fn send_sigterm(&self, pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, libc::SIGTERM) == 0 }
    }

    #[cfg(unix)]
    fn is_process_alive(&self, pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    #[cfg(not(unix))]
    fn is_process_alive(&self, _pid: u32) -> bool {
        true
    }

    /// Clean up server resources (PID file)
    pub fn cleanup(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.remove_pid()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;

    #[test]
    fn test_server_lifecycle() {
        let app_context = AppContext::builder().build();
        let tsk_env = app_context.tsk_env();

        let lifecycle = ServerLifecycle::new(tsk_env);

        assert!(!lifecycle.is_server_running());

        lifecycle.write_pid().unwrap();
        assert!(lifecycle.is_server_running());

        let pid = lifecycle.read_pid().unwrap();
        assert_eq!(pid, process::id());

        lifecycle.cleanup().unwrap();
        assert!(!lifecycle.is_server_running());
    }
}
