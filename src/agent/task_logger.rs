use crate::agent::log_line::{Level, LogLine};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

/// Consolidates task-scoped logging into a single type.
///
/// Keeps the file handle open instead of re-opening on every write.
///
/// Uses `Mutex` (not `RefCell`) because `ProxyManager::acquire_proxy()` captures `&TaskLogger`
/// in an async closure inside `tokio::spawn`, requiring `Send`/`Sync`.
pub struct TaskLogger {
    file: Mutex<Option<File>>,
    suppress_stdout: bool,
}

impl TaskLogger {
    /// Wraps a pre-opened file handle.
    pub fn new(file: File, suppress_stdout: bool) -> Self {
        Self {
            file: Mutex::new(Some(file)),
            suppress_stdout,
        }
    }

    /// Opens `path` in append mode, creating parent dirs as needed.
    pub fn from_path(path: &Path, suppress_stdout: bool) -> Self {
        let file = Self::open_append(path);
        Self {
            file: Mutex::new(file),
            suppress_stdout,
        }
    }

    /// No file -- always prints to stdout. Used by `tsk docker build` which has no task.
    pub fn no_file() -> Self {
        Self {
            file: Mutex::new(None),
            suppress_stdout: false,
        }
    }

    /// JSON-serializes `log_line` to the file. Prints to stdout/stderr when `!suppress_stdout`.
    pub fn log(&self, log_line: LogLine) {
        if let Ok(mut guard) = self.file.lock()
            && let Some(f) = guard.as_mut()
            && let Ok(json) = serde_json::to_string(&log_line)
        {
            let _ = writeln!(f, "{json}");
        }

        if !self.suppress_stdout {
            match &log_line {
                LogLine::Message {
                    level: Level::Warning | Level::Error,
                    message,
                    ..
                } => eprintln!("{message}"),
                LogLine::Message { message, .. } => println!("{message}"),
                _ => {}
            }
        }
    }

    fn open_append(path: &Path) -> Option<File> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::TempDir;

    #[test]
    fn test_log_writes_to_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.log");
        let file = File::create(&path).unwrap();
        let logger = TaskLogger::new(file, true);

        logger.log(LogLine::tsk_message("hello from test"));

        let mut contents = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();

        let parsed: LogLine = serde_json::from_str(contents.trim()).unwrap();
        if let LogLine::Message { message, tags, .. } = parsed {
            assert_eq!(message, "hello from test");
            assert_eq!(tags, vec!["tsk"]);
        } else {
            panic!("Expected Message variant");
        }
    }

    #[test]
    fn test_no_file_does_not_panic() {
        let logger = TaskLogger::no_file();
        logger.log(LogLine::tsk_message("should not panic"));
    }

    #[test]
    fn test_from_path_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("dirs").join("test.log");

        let logger = TaskLogger::from_path(&path, true);
        logger.log(LogLine::tsk_message("nested test"));

        assert!(path.exists());

        let mut contents = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert!(contents.contains("nested test"));
    }
}
