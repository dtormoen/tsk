use std::env;
use std::path::PathBuf;

/// Configuration for environment-dependent settings
///
/// This struct centralizes environment variable access, providing
/// a thread-safe way to override values in tests without using
/// unsafe `std::env::set_var` operations.
#[derive(Clone, Debug)]
pub struct Config {
    /// Path to Claude configuration directory (defaults to $HOME/.claude)
    claude_config_dir: PathBuf,
    /// Editor command for opening files (defaults to $EDITOR or "vi")
    editor: String,
    /// Terminal type for capability detection (from $TERM)
    terminal_type: Option<String>,
}

impl Config {
    /// Creates a new Config from environment variables with sensible defaults
    pub fn new() -> Self {
        let home_dir = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let claude_config_dir = PathBuf::from(home_dir).join(".claude");

        let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());

        let terminal_type = env::var("TERM").ok();

        Self {
            claude_config_dir,
            editor,
            terminal_type,
        }
    }

    /// Returns a builder for creating a custom Config
    #[allow(dead_code)] // Used in tests for creating custom configs
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }

    /// Gets the Claude configuration directory path
    pub fn claude_config_dir(&self) -> &PathBuf {
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
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating Config instances with custom values
#[allow(dead_code)] // Used in tests for creating custom configs
pub struct ConfigBuilder {
    claude_config_dir: Option<PathBuf>,
    editor: Option<String>,
    terminal_type: Option<Option<String>>,
}

impl ConfigBuilder {
    /// Creates a new ConfigBuilder
    #[allow(dead_code)] // Used in tests
    pub fn new() -> Self {
        Self {
            claude_config_dir: None,
            editor: None,
            terminal_type: None,
        }
    }

    /// Sets the Claude configuration directory
    #[allow(dead_code)] // Used in tests
    pub fn with_claude_config_dir(mut self, dir: PathBuf) -> Self {
        self.claude_config_dir = Some(dir);
        self
    }

    /// Sets the editor command
    #[allow(dead_code)] // Used in tests
    pub fn with_editor(mut self, editor: String) -> Self {
        self.editor = Some(editor);
        self
    }

    /// Sets the terminal type
    #[allow(dead_code)] // Used in tests
    pub fn with_terminal_type(mut self, terminal_type: Option<String>) -> Self {
        self.terminal_type = Some(terminal_type);
        self
    }

    /// Builds the Config instance
    #[allow(dead_code)] // Used in tests
    pub fn build(self) -> Config {
        let default = Config::new();

        Config {
            claude_config_dir: self.claude_config_dir.unwrap_or(default.claude_config_dir),
            editor: self.editor.unwrap_or(default.editor),
            terminal_type: self.terminal_type.unwrap_or(default.terminal_type),
        }
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_config_new_with_defaults() {
        // This test relies on actual environment variables
        // In a clean environment, it should use defaults
        let config = Config::new();

        // Check that the fields are populated
        assert!(!config.editor().is_empty());
        assert!(!config.claude_config_dir().as_os_str().is_empty());
    }

    #[test]
    fn test_config_builder() {
        let temp_dir = TempDir::new().unwrap();
        let claude_dir = temp_dir.path().join(".claude");

        let config = Config::builder()
            .with_claude_config_dir(claude_dir.clone())
            .with_editor("emacs".to_string())
            .with_terminal_type(Some("xterm-256color".to_string()))
            .build();

        assert_eq!(config.claude_config_dir(), &claude_dir);
        assert_eq!(config.editor(), "emacs");
        assert_eq!(config.terminal_type(), Some("xterm-256color"));
    }

    #[test]
    fn test_config_builder_partial() {
        let config = Config::builder().with_editor("nano".to_string()).build();

        assert_eq!(config.editor(), "nano");
        // Other fields should have defaults
        assert!(!config.claude_config_dir().as_os_str().is_empty());
    }

    #[test]
    fn test_config_builder_none_terminal() {
        let config = Config::builder().with_terminal_type(None).build();

        assert_eq!(config.terminal_type(), None);
    }
}
