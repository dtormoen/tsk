use serde::{Deserialize, Serialize};
use std::fmt;

/// A structured log line produced by agent log processors.
///
/// Each variant represents a semantically distinct data shape.
/// Agents emit the subset that fits their capabilities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogLine {
    /// General-purpose log line: tool invocations, tool results, agent text, errors.
    Message {
        #[serde(default, skip_serializing_if = "Level::is_default")]
        level: Level,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
        message: String,
    },
    /// Structured todo list update with per-item status.
    Todo {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
        items: Vec<TodoItem>,
    },
    /// Final task result with structured metadata.
    Summary {
        success: bool,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        num_turns: Option<u64>,
    },
}

/// Severity level for Message log lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl Level {
    /// Returns true if this is the default level (Info).
    /// Used by serde skip_serializing_if.
    pub fn is_default(&self) -> bool {
        matches!(self, Level::Info)
    }
}

/// A single item in a structured todo list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// Status of a todo item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl LogLine {
    /// Create a Message with Info level.
    pub fn message(tags: Vec<String>, tool: Option<String>, message: String) -> Self {
        LogLine::Message {
            level: Level::Info,
            tags,
            tool,
            message,
        }
    }

    /// Create a Message with Error level.
    pub fn error(tags: Vec<String>, tool: Option<String>, message: String) -> Self {
        LogLine::Message {
            level: Level::Error,
            tags,
            tool,
            message,
        }
    }

    /// Create a Message with Success level.
    pub fn success(tags: Vec<String>, tool: Option<String>, message: String) -> Self {
        LogLine::Message {
            level: Level::Success,
            tags,
            tool,
            message,
        }
    }

    /// Create a Message with Warning level.
    pub fn warning(tags: Vec<String>, tool: Option<String>, message: String) -> Self {
        LogLine::Message {
            level: Level::Warning,
            tags,
            tool,
            message,
        }
    }

    /// Create an infrastructure message with the "tsk" tag (Info level).
    pub fn tsk_message(message: impl Into<String>) -> Self {
        Self::message(vec!["tsk".into()], None, message.into())
    }

    /// Create an infrastructure success message with the "tsk" tag.
    pub fn tsk_success(message: impl Into<String>) -> Self {
        Self::success(vec!["tsk".into()], None, message.into())
    }

    /// Create an infrastructure warning with the "tsk" tag.
    pub fn tsk_warning(message: impl Into<String>) -> Self {
        Self::warning(vec!["tsk".into()], None, message.into())
    }

    /// Create a Todo log line.
    pub fn todo(tags: Vec<String>, items: Vec<TodoItem>) -> Self {
        LogLine::Todo { tags, items }
    }

    /// Create a Summary log line.
    pub fn summary(
        success: bool,
        message: String,
        cost_usd: Option<f64>,
        duration_ms: Option<u64>,
        num_turns: Option<u64>,
    ) -> Self {
        LogLine::Summary {
            success,
            message,
            cost_usd,
            duration_ms,
            num_turns,
        }
    }
}

impl fmt::Display for LogLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogLine::Message {
                tags,
                tool,
                message,
                ..
            } => {
                for tag in tags {
                    write!(f, "[{tag}]")?;
                }
                if !tags.is_empty() {
                    write!(f, " ")?;
                }
                if let Some(tool) = tool {
                    write!(f, "{tool}: ")?;
                }
                write!(f, "{message}")
            }
            LogLine::Todo { tags, items } => {
                for tag in tags {
                    write!(f, "[{tag}]")?;
                }
                if !tags.is_empty() {
                    write!(f, " ")?;
                }
                write!(f, "TodoWrite:")?;
                let completed = items
                    .iter()
                    .filter(|i| i.status == TodoStatus::Completed)
                    .count();
                for item in items {
                    let (checkbox, text) = match item.status {
                        TodoStatus::Completed => ("[x]", item.content.as_str()),
                        TodoStatus::InProgress => {
                            ("[~]", item.active_form.as_deref().unwrap_or(&item.content))
                        }
                        TodoStatus::Pending => ("[ ]", item.content.as_str()),
                    };
                    write!(f, " {checkbox} {text},")?;
                }
                write!(f, " ({}/{} done)", completed, items.len())
            }
            LogLine::Summary {
                success,
                message,
                cost_usd,
                duration_ms,
                num_turns,
            } => {
                let status = if *success { "success" } else { "failed" };
                write!(f, "Result: {status} | {message}")?;
                if let Some(cost) = cost_usd {
                    write!(f, " | ${cost:.2}")?;
                }
                if let Some(ms) = duration_ms {
                    let secs = ms / 1000;
                    write!(f, " | {secs}s")?;
                }
                if let Some(turns) = num_turns {
                    write!(f, " | {turns} turns")?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let line = LogLine::message(vec![], Some("Bash".into()), "Running: cargo test".into());
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(
            json,
            r#"{"type":"message","tool":"Bash","message":"Running: cargo test"}"#
        );

        // Round-trip
        let parsed: LogLine = serde_json::from_str(&json).unwrap();
        if let LogLine::Message {
            tool,
            message,
            level,
            tags,
        } = parsed
        {
            assert_eq!(tool, Some("Bash".to_string()));
            assert_eq!(message, "Running: cargo test");
            assert_eq!(level, Level::Info);
            assert!(tags.is_empty());
        } else {
            panic!("Expected Message variant");
        }
    }

    #[test]
    fn test_error_serialization() {
        let line = LogLine::error(vec![], Some("Bash".into()), "Tests failed".into());
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains(r#""level":"error""#));
        assert!(json.contains(r#""tool":"Bash""#));
    }

    #[test]
    fn test_todo_serialization() {
        let items = vec![
            TodoItem {
                content: "Update imports".into(),
                status: TodoStatus::Completed,
                active_form: None,
                priority: None,
            },
            TodoItem {
                content: "Analyzing code".into(),
                status: TodoStatus::InProgress,
                active_form: Some("Analyzing code".into()),
                priority: None,
            },
            TodoItem {
                content: "Write tests".into(),
                status: TodoStatus::Pending,
                active_form: None,
                priority: None,
            },
        ];
        let line = LogLine::todo(vec![], items);
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains(r#""type":"todo""#));

        let parsed: LogLine = serde_json::from_str(&json).unwrap();
        if let LogLine::Todo { items, .. } = parsed {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].status, TodoStatus::Completed);
            assert_eq!(items[1].status, TodoStatus::InProgress);
        } else {
            panic!("Expected Todo variant");
        }
    }

    #[test]
    fn test_summary_serialization() {
        let line = LogLine::summary(
            true,
            "Task completed".into(),
            Some(0.15),
            Some(45000),
            Some(12),
        );
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains(r#""type":"summary""#));
        assert!(json.contains(r#""success":true"#));
        assert!(json.contains(r#""cost_usd":0.15"#));
    }

    #[test]
    fn test_summary_compact_serialization() {
        // Fields with None should be omitted
        let line = LogLine::summary(false, "Task failed".into(), None, None, None);
        let json = serde_json::to_string(&line).unwrap();
        assert!(!json.contains("cost_usd"));
        assert!(!json.contains("duration_ms"));
        assert!(!json.contains("num_turns"));
    }

    #[test]
    fn test_display_message() {
        let line = LogLine::message(
            vec!["opus-4".into(), "software-architect".into()],
            Some("Bash".into()),
            "Running: cargo test".into(),
        );
        assert_eq!(
            line.to_string(),
            "[opus-4][software-architect] Bash: Running: cargo test"
        );
    }

    #[test]
    fn test_display_message_no_tags_no_tool() {
        let line = LogLine::message(vec![], None, "Hello world".into());
        assert_eq!(line.to_string(), "Hello world");
    }

    #[test]
    fn test_display_todo() {
        let items = vec![
            TodoItem {
                content: "Done task".into(),
                status: TodoStatus::Completed,
                active_form: None,
                priority: None,
            },
            TodoItem {
                content: "Working".into(),
                status: TodoStatus::InProgress,
                active_form: Some("Working on it".into()),
                priority: None,
            },
            TodoItem {
                content: "Not started".into(),
                status: TodoStatus::Pending,
                active_form: None,
                priority: None,
            },
        ];
        let line = LogLine::todo(vec![], items);
        let display = line.to_string();
        assert!(display.contains("TodoWrite:"));
        assert!(display.contains("[x] Done task,"));
        assert!(display.contains("[~] Working on it,"));
        assert!(display.contains("[ ] Not started,"));
        assert!(display.contains("(1/3 done)"));
    }

    #[test]
    fn test_display_summary() {
        let line = LogLine::summary(
            true,
            "Task completed".into(),
            Some(0.15),
            Some(45000),
            Some(12),
        );
        assert_eq!(
            line.to_string(),
            "Result: success | Task completed | $0.15 | 45s | 12 turns"
        );
    }

    #[test]
    fn test_level_is_default() {
        assert!(Level::Info.is_default());
        assert!(!Level::Error.is_default());
        assert!(!Level::Success.is_default());
        assert!(!Level::Warning.is_default());
    }
}
