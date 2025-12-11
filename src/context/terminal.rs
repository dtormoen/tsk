use crate::context::tsk_env::TskEnv;
use is_terminal::IsTerminal;
use std::io::{self, Write};
#[cfg(test)]
use std::sync::Arc;
use std::sync::Mutex;

/// Trait for terminal operations
pub trait TerminalOperations: Send + Sync {
    /// Set the terminal window title
    fn set_title(&self, title: &str);

    /// Restore the original terminal title
    fn restore_title(&self);
}

/// Default terminal operations implementation
pub struct DefaultTerminalOperations {
    state: Mutex<TerminalState>,
    #[cfg(test)]
    #[allow(dead_code)]
    tsk_env: Option<Arc<TskEnv>>,
}

struct TerminalState {
    supported: bool,
    original_title: Option<String>,
}

impl DefaultTerminalOperations {
    /// Create a new terminal operations instance
    pub fn new() -> Self {
        let supported = Self::is_supported(None);

        Self {
            state: Mutex::new(TerminalState {
                supported,
                original_title: None,
            }),
            #[cfg(test)]
            tsk_env: None,
        }
    }

    /// Create a new terminal operations instance with TSK environment
    #[cfg(test)]
    pub fn with_tsk_env(tsk_env: Arc<TskEnv>) -> Self {
        let supported = Self::is_supported(Some(&tsk_env));

        Self {
            state: Mutex::new(TerminalState {
                supported,
                original_title: None,
            }),
            #[cfg(test)]
            tsk_env: Some(tsk_env),
        }
    }

    /// Check if terminal title updates are supported
    fn is_supported(tsk_env: Option<&TskEnv>) -> bool {
        // Check if we're in a TTY
        if !std::io::stdout().is_terminal() {
            return false;
        }

        // Check for common terminal environment variables
        let term = if let Some(env) = tsk_env {
            env.terminal_type().map(|s| s.to_string())
        } else {
            std::env::var("TERM").ok()
        };

        if let Some(term_type) = term {
            // Most modern terminals support title changes
            // Exclude known non-supporting terminals
            !matches!(term_type.as_str(), "dumb" | "unknown")
        } else {
            false
        }
    }

    /// Write the terminal title using OSC sequences
    fn write_title(title: &str) {
        // Use OSC (Operating System Command) sequences
        // OSC 0 sets both window and icon title
        // Format: ESC]0;title BEL
        let _ = write!(io::stdout(), "\x1b]0;{title}\x07");
        let _ = io::stdout().flush();
    }
}

impl TerminalOperations for DefaultTerminalOperations {
    fn set_title(&self, title: &str) {
        let mut state = self.state.lock().unwrap();

        if !state.supported {
            return;
        }

        // Store the original title on first use
        if state.original_title.is_none() {
            // Note: There's no portable way to get the current title
            // We'll just store a default to restore
            state.original_title = Some("Terminal".to_string());
        }

        Self::write_title(title);
    }

    fn restore_title(&self) {
        let state = self.state.lock().unwrap();

        if !state.supported {
            return;
        }

        if let Some(ref original) = state.original_title {
            Self::write_title(original);
        }
    }
}

impl Drop for DefaultTerminalOperations {
    fn drop(&mut self) {
        // Restore title when the object is dropped
        self.restore_title();
    }
}

impl Default for DefaultTerminalOperations {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_operations_creation() {
        let terminal_ops = DefaultTerminalOperations::new();
        // In test environment, it might not be supported
        // Just ensure it doesn't panic
        let state = terminal_ops.state.lock().unwrap();
        assert!(state.original_title.is_none());
    }

    #[test]
    fn test_set_title_no_panic() {
        let terminal_ops = DefaultTerminalOperations::new();
        // Should not panic even if not supported
        terminal_ops.set_title("Test Title");
        terminal_ops.restore_title();
    }

    #[test]
    fn test_drop_restores_title() {
        {
            let terminal_ops = DefaultTerminalOperations::new();
            terminal_ops.set_title("Temporary Title");
            // Terminal title should be restored when going out of scope
        }
        // Title should have been restored by Drop implementation
    }

    #[test]
    fn test_terminal_support_detection() {
        use crate::context::tsk_env::TskEnv;

        // Test with xterm-256color terminal
        let env_xterm = TskEnv::builder()
            .with_terminal_type(Some("xterm-256color".to_string()))
            .build()
            .unwrap();
        assert!(
            DefaultTerminalOperations::is_supported(Some(&env_xterm))
                || !std::io::stdout().is_terminal()
        );

        // Test with dumb terminal
        let env_dumb = TskEnv::builder()
            .with_terminal_type(Some("dumb".to_string()))
            .build()
            .unwrap();
        assert!(!DefaultTerminalOperations::is_supported(Some(&env_dumb)));

        // Test with no terminal type set
        let env_none = TskEnv::builder().with_terminal_type(None).build().unwrap();
        assert!(!DefaultTerminalOperations::is_supported(Some(&env_none)));

        // Test terminal operations with TSK environment
        let terminal_with_env = DefaultTerminalOperations::with_tsk_env(Arc::new(env_xterm));
        // Should not panic
        terminal_with_env.set_title("Test");
        terminal_with_env.restore_title();
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let terminal_ops = Arc::new(DefaultTerminalOperations::new());
        let mut handles = vec![];

        for i in 0..5 {
            let ops = Arc::clone(&terminal_ops);
            let handle = thread::spawn(move || {
                ops.set_title(&format!("Thread {i}"));
                ops.restore_title();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }
}
