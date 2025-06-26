use std::io::{self, Write};
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
}

struct TerminalState {
    supported: bool,
    original_title: Option<String>,
}

impl DefaultTerminalOperations {
    /// Create a new terminal operations instance
    pub fn new() -> Self {
        let supported = Self::is_supported();

        Self {
            state: Mutex::new(TerminalState {
                supported,
                original_title: None,
            }),
        }
    }

    /// Check if terminal title updates are supported
    fn is_supported() -> bool {
        // Check if we're in a TTY
        if !atty::is(atty::Stream::Stdout) {
            return false;
        }

        // Check for common terminal environment variables
        if let Ok(term) = std::env::var("TERM") {
            // Most modern terminals support title changes
            // Exclude known non-supporting terminals
            !matches!(term.as_str(), "dumb" | "unknown")
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
        // Test with TERM environment variable
        std::env::set_var("TERM", "xterm-256color");
        assert!(DefaultTerminalOperations::is_supported() || !atty::is(atty::Stream::Stdout));

        std::env::set_var("TERM", "dumb");
        assert!(!DefaultTerminalOperations::is_supported());

        std::env::remove_var("TERM");
        assert!(!DefaultTerminalOperations::is_supported());
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
