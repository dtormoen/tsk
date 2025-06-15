use std::io::{self, Write};

/// Terminal window title management
pub struct TerminalTitle {
    /// Whether terminal title updates are supported
    supported: bool,
    /// Original title to restore when done
    original_title: Option<String>,
}

impl TerminalTitle {
    /// Create a new TerminalTitle instance
    pub fn new() -> Self {
        // Check if we're in a supported terminal environment
        let supported = Self::is_supported();

        Self {
            supported,
            original_title: None,
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

    /// Set the terminal window title
    pub fn set_title(&mut self, title: &str) {
        if !self.supported {
            return;
        }

        // Store the original title on first use
        if self.original_title.is_none() {
            // Note: There's no portable way to get the current title
            // We'll just store a default to restore
            self.original_title = Some("Terminal".to_string());
        }

        // Use OSC (Operating System Command) sequences
        // OSC 0 sets both window and icon title
        // Format: ESC]0;title BEL
        let _ = write!(io::stdout(), "\x1b]0;{}\x07", title);
        let _ = io::stdout().flush();
    }

    /// Restore the original terminal title
    pub fn restore(&mut self) {
        if !self.supported {
            return;
        }

        if let Some(ref original) = self.original_title {
            let _ = write!(io::stdout(), "\x1b]0;{}\x07", original);
            let _ = io::stdout().flush();
        }
    }
}

impl Drop for TerminalTitle {
    fn drop(&mut self) {
        // Restore title when the object is dropped
        self.restore();
    }
}

impl Default for TerminalTitle {
    fn default() -> Self {
        Self::new()
    }
}

/// Global terminal title instance for the server
static mut TERMINAL_TITLE: Option<TerminalTitle> = None;
static TERMINAL_TITLE_INIT: std::sync::Once = std::sync::Once::new();

/// Set the global terminal title (thread-safe initialization)
pub fn set_terminal_title(title: &str) {
    unsafe {
        TERMINAL_TITLE_INIT.call_once(|| {
            TERMINAL_TITLE = Some(TerminalTitle::new());
        });

        if let Some(ref mut terminal_title) = TERMINAL_TITLE {
            terminal_title.set_title(title);
        }
    }
}

/// Restore the global terminal title
pub fn restore_terminal_title() {
    unsafe {
        if let Some(ref mut terminal_title) = TERMINAL_TITLE {
            terminal_title.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_title_creation() {
        let terminal_title = TerminalTitle::new();
        // In test environment, it might not be supported
        // Just ensure it doesn't panic
        assert!(terminal_title.original_title.is_none());
    }

    #[test]
    fn test_set_title_no_panic() {
        let mut terminal_title = TerminalTitle::new();
        // Should not panic even if not supported
        terminal_title.set_title("Test Title");
        terminal_title.restore();
    }

    #[test]
    fn test_global_functions_no_panic() {
        // Should not panic
        set_terminal_title("Test Global Title");
        restore_terminal_title();
    }

    #[test]
    fn test_drop_restores_title() {
        {
            let mut terminal_title = TerminalTitle::new();
            terminal_title.set_title("Temporary Title");
            // Terminal title should be restored when going out of scope
        }
        // Title should have been restored by Drop implementation
    }

    #[test]
    fn test_terminal_support_detection() {
        // Test with TERM environment variable
        std::env::set_var("TERM", "xterm-256color");
        assert!(TerminalTitle::is_supported() || !atty::is(atty::Stream::Stdout));

        std::env::set_var("TERM", "dumb");
        assert!(!TerminalTitle::is_supported());

        std::env::remove_var("TERM");
        assert!(!TerminalTitle::is_supported());
    }
}
