use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::{LogProcessor, TaskResult};

/// Represents a message from Claude Code's JSON output
#[derive(Debug, Deserialize, Serialize)]
struct ClaudeMessage {
    #[serde(rename = "type")]
    message_type: String,
    message: Option<MessageContent>,
    subtype: Option<String>,
    cost_usd: Option<f64>,
    is_error: Option<bool>,
    duration_ms: Option<u64>,
    duration_api_ms: Option<u64>,
    num_turns: Option<u64>,
    result: Option<String>,
    total_cost: Option<f64>,
    session_id: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "toolUseResult")]
    tool_use_result: Option<Value>,
    summary: Option<String>,
    #[serde(rename = "leafUuid")]
    leaf_uuid: Option<String>,
}

/// Message content structure from Claude Code
#[derive(Debug, Deserialize, Serialize)]
struct MessageContent {
    role: Option<String>,
    content: Option<Value>,
    id: Option<String>,
    #[serde(rename = "type")]
    content_type: Option<String>,
    model: Option<String>,
    stop_reason: Option<String>,
    usage: Option<Usage>,
}

/// Usage information from Claude Code
#[derive(Debug, Deserialize, Serialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    service_tier: Option<String>,
}

/// Todo item structure from Claude Code's TodoWrite tool
#[derive(Debug, Deserialize, Serialize, Clone)]
struct TodoItem {
    content: String,
    status: String,
    #[serde(rename = "activeForm")]
    active_form: Option<String>,
    #[serde(default)]
    priority: Option<String>,
}

/// Claude Code specific log processor that parses and formats JSON output
///
/// This processor provides rich output including:
/// - Tool usage information (Edit, Bash, Write, etc.)
/// - Tool result summaries from user messages
/// - TODO list updates with status indicators
/// - Assistant reasoning and conversation
/// - Summary messages
/// - Cost calculations from token usage
///
/// The processor handles non-JSON output gracefully:
/// - Initially prints non-JSON lines as-is (for misconfiguration messages)
/// - Switches to JSON-only mode after the first valid JSON line
pub struct ClaudeCodeLogProcessor {
    full_log: Vec<String>,
    final_result: Option<TaskResult>,
    json_mode_active: bool,
}

impl ClaudeCodeLogProcessor {
    /// Creates a new ClaudeCodeLogProcessor
    pub fn new() -> Self {
        Self {
            full_log: Vec::new(),
            final_result: None,
            json_mode_active: false,
        }
    }

    /// Formats a Claude message based on its type
    fn format_message(&mut self, msg: ClaudeMessage) -> Option<String> {
        match msg.message_type.as_str() {
            "assistant" => self.format_assistant_message(msg),
            "user" => self.format_user_message(&msg),
            "result" => self.format_result_message(msg),
            "summary" => {
                // Summary messages indicate task completion
                if let Some(summary_text) = msg.summary {
                    // Store as final result if not already set
                    if self.final_result.is_none() {
                        self.final_result = Some(TaskResult {
                            success: true,
                            message: summary_text.clone(),
                            cost_usd: None,
                            duration_ms: None,
                        });
                    }
                    Some(format!("✅ Task Complete: {summary_text}"))
                } else {
                    Some("✅ Task Complete".to_string())
                }
            }
            other_type => {
                // For other message types, just show a brief indicator
                Some(format!("📋 [{other_type}]"))
            }
        }
    }

    /// Formats a user message with tool result information
    fn format_user_message(&self, msg: &ClaudeMessage) -> Option<String> {
        // Check if this is a tool result message
        if let Some(message) = &msg.message
            && let Some(content) = &message.content
        {
            // Check if content is an array with tool_result
            if let Value::Array(contents) = content {
                for item in contents {
                    if let Some("tool_result") = item.get("type").and_then(|t| t.as_str()) {
                        // Try to determine which tool was used from toolUseResult
                        if let Some(tool_result) = msg.tool_use_result.as_ref() {
                            return Some(self.format_tool_result(tool_result));
                        }
                        // Generic tool result if we couldn't identify the type
                        return Some("🔧 Tool result".to_string());
                    }
                }
            }
            // Check if it's a regular user message with string content
            else if let Value::String(text) = content {
                let first_line = text.lines().next().unwrap_or("").trim();
                if first_line.starts_with("# ") {
                    return Some(format!("👤 User: {}", first_line.trim_start_matches("# ")));
                } else if first_line.len() > 60 {
                    return Some(format!("👤 User: {}...", &first_line[..60]));
                } else if !first_line.is_empty() {
                    return Some(format!("👤 User: {first_line}"));
                }
            }
        }

        // Default fallback
        Some("👤 [user]".to_string())
    }

    /// Formats tool result based on its content
    fn format_tool_result(&self, tool_result: &Value) -> String {
        // Check for specific result types to identify the tool
        if tool_result.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(file_info) = tool_result.get("file")
                && let Some(path) = file_info.get("filePath").and_then(|p| p.as_str())
            {
                let filename = path.rsplit('/').next().unwrap_or(path);
                return format!("📖 Read result: {filename}");
            }
            return "📖 Read result".to_string();
        }

        if tool_result.get("filePath").is_some() {
            return "✏️ Edit result".to_string();
        }

        if tool_result.get("oldTodos").is_some() || tool_result.get("newTodos").is_some() {
            // For TodoWrite results, show the todo update
            if let Some(new_todos) = tool_result.get("newTodos")
                && let Ok(todos) = serde_json::from_value::<Vec<TodoItem>>(new_todos.clone())
            {
                return format!("📝 TodoWrite result - {} todos", todos.len());
            }
            return "📝 TodoWrite result".to_string();
        }

        if let Some(filenames) = tool_result.get("filenames").and_then(|v| v.as_array()) {
            let count = filenames.len();
            return format!(
                "🔍 Search result: Found {} file{}",
                count,
                if count == 1 { "" } else { "s" }
            );
        }

        if let Some(stdout) = tool_result.get("stdout").and_then(|v| v.as_str()) {
            if stdout.contains("test result: ok") {
                return "🖥️ Bash result: Tests passed ✅".to_string();
            } else if stdout.contains("test result: FAILED") {
                return "🖥️ Bash result: Tests failed ❌".to_string();
            } else if stdout.trim().is_empty() {
                return "🖥️ Bash result: Command completed".to_string();
            } else {
                let first_line = stdout.lines().next().unwrap_or("").trim();
                if first_line.len() > 40 {
                    return format!("🖥️ Bash result: {}...", &first_line[..40]);
                } else {
                    return format!("🖥️ Bash result: {first_line}");
                }
            }
        }

        if let Some(error) = tool_result.get("error").and_then(|v| v.as_str()) {
            return format!("❌ Tool error: {error}");
        }

        if tool_result.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
            return "❌ Tool error occurred".to_string();
        }

        "🔧 Tool result: Completed".to_string()
    }

    /// Formats an assistant message, extracting text content, tool uses, and todo updates
    fn format_assistant_message(&self, msg: ClaudeMessage) -> Option<String> {
        if let Some(message) = msg.message {
            if let Some(content) = message.content {
                match content {
                    Value::Array(contents) => {
                        let mut output = String::new();

                        for item in contents {
                            // Check for tool use
                            if let Some("tool_use") = item.get("type").and_then(|t| t.as_str())
                                && let Some(tool_name) = item.get("name").and_then(|n| n.as_str())
                            {
                                let tool_output = self.format_tool_use(tool_name, &item);
                                if !tool_output.is_empty() {
                                    output.push_str(&tool_output);
                                }
                            }

                            // Process regular text content
                            if let Some(text) = item.get("text").and_then(|t| t.as_str())
                                && !text.trim().is_empty()
                            {
                                if !output.is_empty() {
                                    output.push('\n');
                                }
                                output.push_str(&format!("🤖 {text}"));
                            }
                        }

                        if !output.is_empty() {
                            Some(output.trim_end().to_string())
                        } else {
                            None
                        }
                    }
                    Value::String(text) => Some(format!("🤖 {text}")),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Formats a tool use based on its name and input
    fn format_tool_use(&self, tool_name: &str, item: &Value) -> String {
        match tool_name {
            "TodoWrite" => {
                if let Some(input) = item.get("input")
                    && let Some(todos) = input.get("todos")
                    && let Ok(todo_items) = serde_json::from_value::<Vec<TodoItem>>(todos.clone())
                {
                    self.format_todo_update(&todo_items)
                } else {
                    "📝 Using TodoWrite\n".to_string()
                }
            }
            "Read" => {
                if let Some(input) = item.get("input")
                    && let Some(file_path) = input.get("file_path").and_then(|f| f.as_str())
                {
                    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
                    format!("📖 Reading {file_name}\n")
                } else {
                    "📖 Reading file\n".to_string()
                }
            }
            "LS" => {
                if let Some(input) = item.get("input")
                    && let Some(path) = input.get("path").and_then(|p| p.as_str())
                {
                    format!("📂 Listing {path}\n")
                } else {
                    "📂 Listing directory\n".to_string()
                }
            }
            "NotebookRead" => "📓 Reading notebook\n".to_string(),
            "Edit" | "MultiEdit" => {
                if let Some(input) = item.get("input")
                    && let Some(file_path) = input.get("file_path").and_then(|f| f.as_str())
                {
                    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
                    if tool_name == "MultiEdit" {
                        if let Some(edits) = input.get("edits").and_then(|e| e.as_array()) {
                            format!("🔧 Editing {file_name} ({} changes)\n", edits.len())
                        } else {
                            format!("🔧 Editing {file_name}\n")
                        }
                    } else {
                        format!("🔧 Editing {file_name}\n")
                    }
                } else {
                    format!("🔧 Using {tool_name}\n")
                }
            }
            "Bash" => {
                if let Some(input) = item.get("input")
                    && let Some(cmd) = input.get("command").and_then(|c| c.as_str())
                {
                    let cmd_preview = if cmd.len() > 60 {
                        format!("{}...", &cmd[..60])
                    } else {
                        cmd.to_string()
                    };
                    format!("🖥️ Running: {cmd_preview}\n")
                } else {
                    "🖥️ Running command\n".to_string()
                }
            }
            "Write" => {
                if let Some(input) = item.get("input")
                    && let Some(file_path) = input.get("file_path").and_then(|f| f.as_str())
                {
                    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
                    format!("📝 Writing {file_name}\n")
                } else {
                    "📝 Writing file\n".to_string()
                }
            }
            "Grep" => {
                if let Some(input) = item.get("input")
                    && let Some(pattern) = input.get("pattern").and_then(|p| p.as_str())
                {
                    format!("🔍 Searching for: {pattern}\n")
                } else {
                    "🔍 Searching\n".to_string()
                }
            }
            "WebSearch" => {
                if let Some(input) = item.get("input")
                    && let Some(query) = input.get("query").and_then(|q| q.as_str())
                {
                    format!("🌐 Web search: {query}\n")
                } else {
                    "🌐 Web search\n".to_string()
                }
            }
            _ => format!("🔧 Using {tool_name}\n"),
        }
    }

    /// Formats a todo list update with status indicators and summary
    fn format_todo_update(&self, todos: &[TodoItem]) -> String {
        let mut output = String::new();
        output.push_str("📝 TODO Update:\n");
        output.push_str(&"─".repeat(60));
        output.push('\n');

        // Display todos with status emoji
        for (idx, todo) in todos.iter().enumerate() {
            let status_emoji = match todo.status.as_str() {
                "in_progress" => "🔄",
                "pending" => "⏳",
                "completed" => "✅",
                _ => "⏳",
            };

            let priority_emoji = if let Some(ref priority) = todo.priority {
                self.get_priority_emoji(priority)
            } else {
                ""
            };

            // Use activeForm if available, otherwise use content
            let display_text = if todo.status == "in_progress" {
                todo.active_form.as_ref().unwrap_or(&todo.content)
            } else {
                &todo.content
            };

            output.push_str(&format!(
                "{} {} [{}] {}\n",
                status_emoji,
                priority_emoji,
                idx + 1,
                display_text
            ));
        }

        output.push_str(&"─".repeat(60));
        output.push('\n');

        // Add summary
        let total = todos.len();
        let completed = todos.iter().filter(|t| t.status == "completed").count();
        let in_progress = todos.iter().filter(|t| t.status == "in_progress").count();
        let pending = todos.iter().filter(|t| t.status == "pending").count();

        output.push_str(&format!(
            "Summary: {total} total | {completed} completed | {in_progress} in progress | {pending} pending\n"
        ));
        output.push_str(&"─".repeat(60));

        output
    }

    /// Returns an emoji for the given priority level
    fn get_priority_emoji(&self, priority: &str) -> &'static str {
        match priority {
            "high" => "🔴",
            "medium" => "🟡",
            "low" => "🟢",
            _ => "⚪",
        }
    }

    /// Formats duration in milliseconds to a human-readable string
    fn format_duration(&self, duration_ms: u64) -> String {
        let total_seconds = duration_ms / 1000;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        let mut parts = Vec::new();

        if hours > 0 {
            parts.push(format!(
                "{} hour{}",
                hours,
                if hours == 1 { "" } else { "s" }
            ));
        }

        if minutes > 0 {
            parts.push(format!(
                "{} minute{}",
                minutes,
                if minutes == 1 { "" } else { "s" }
            ));
        }

        if seconds > 0 || parts.is_empty() {
            parts.push(format!(
                "{} second{}",
                seconds,
                if seconds == 1 { "" } else { "s" }
            ));
        }

        parts.join(", ")
    }

    /// Formats a result message and stores the task result
    fn format_result_message(&mut self, msg: ClaudeMessage) -> Option<String> {
        // Parse and store the result status
        if let Some(subtype) = &msg.subtype {
            let success = subtype == "success";
            let message = msg.result.clone().unwrap_or_else(|| {
                if success {
                    "Task completed successfully".to_string()
                } else {
                    "Task failed".to_string()
                }
            });

            // Calculate cost from usage if not directly provided
            let cost_usd = msg.cost_usd.or_else(|| {
                if let Some(message_content) = &msg.message {
                    if let Some(usage) = &message_content.usage {
                        self.calculate_cost_from_usage(usage)
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

            self.final_result = Some(TaskResult {
                success,
                message: message.clone(),
                cost_usd,
                duration_ms: msg.duration_ms,
            });

            // Format a nice summary
            let mut output = String::new();
            output.push('\n');
            output.push_str(&"─".repeat(60));
            output.push('\n');

            let status_emoji = if success { "✅" } else { "❌" };
            output.push_str(&format!("{status_emoji} Task Result: {subtype}\n"));

            if let Some(cost) = cost_usd {
                output.push_str(&format!("💰 Cost: ${cost:.2}\n"));
            }

            if let Some(duration) = msg.duration_ms {
                output.push_str(&format!(
                    "⏱️ Duration: {}\n",
                    self.format_duration(duration)
                ));
            }

            if let Some(turns) = msg.num_turns {
                output.push_str(&format!("🔄 Turns: {turns}\n"));
            }

            output.push_str(&"─".repeat(60));

            Some(output)
        } else {
            None
        }
    }

    /// Calculates approximate cost from token usage
    fn calculate_cost_from_usage(&self, usage: &Usage) -> Option<f64> {
        // Approximate pricing for Claude Opus
        const INPUT_COST_PER_1K: f64 = 0.015; // $15 per million tokens
        const OUTPUT_COST_PER_1K: f64 = 0.075; // $75 per million tokens
        const CACHE_WRITE_COST_PER_1K: f64 = 0.01875; // 25% more than input
        const CACHE_READ_COST_PER_1K: f64 = 0.0015; // 90% cheaper than input

        let input_tokens = usage.input_tokens.unwrap_or(0) as f64;
        let output_tokens = usage.output_tokens.unwrap_or(0) as f64;
        let cache_write_tokens = usage.cache_creation_input_tokens.unwrap_or(0) as f64;
        let cache_read_tokens = usage.cache_read_input_tokens.unwrap_or(0) as f64;

        let total_cost = (input_tokens * INPUT_COST_PER_1K / 1000.0)
            + (output_tokens * OUTPUT_COST_PER_1K / 1000.0)
            + (cache_write_tokens * CACHE_WRITE_COST_PER_1K / 1000.0)
            + (cache_read_tokens * CACHE_READ_COST_PER_1K / 1000.0);

        if total_cost > 0.0 {
            Some(total_cost)
        } else {
            None
        }
    }
}

#[async_trait]
impl LogProcessor for ClaudeCodeLogProcessor {
    fn process_line(&mut self, line: &str) -> Option<String> {
        // Store the raw line for the full log
        self.full_log.push(line.to_string());

        // Skip empty lines
        if line.trim().is_empty() {
            return None;
        }

        // Try to parse as JSON
        match serde_json::from_str::<ClaudeMessage>(line) {
            Ok(msg) => {
                // Successfully parsed JSON - activate JSON mode if not already active
                if !self.json_mode_active {
                    self.json_mode_active = true;
                }
                self.format_message(msg)
            }
            Err(_) => {
                if self.json_mode_active {
                    // In JSON mode, show parsing error for non-JSON lines
                    Some("‼️ parsing error".to_string())
                } else {
                    // Before JSON mode is active, pass through non-JSON lines as-is
                    // This allows misconfiguration messages to be displayed
                    Some(line.to_string())
                }
            }
        }
    }

    fn get_full_log(&self) -> String {
        self.full_log.join("\n")
    }

    fn get_final_result(&self) -> Option<&TaskResult> {
        self.final_result.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_todo_update_new_format() {
        let mut processor = ClaudeCodeLogProcessor::new();

        // Test the actual format from Claude Code logs
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "TodoWrite",
                    "input": {
                        "todos": [
                            {"content": "Analyze current usage", "status": "in_progress", "activeForm": "Analyzing current usage"},
                            {"content": "Move file to new location", "status": "pending", "activeForm": "Moving file to new location"},
                            {"content": "Update imports", "status": "completed", "activeForm": "Updating imports"}
                        ]
                    }
                }]
            }
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        let formatted = result.unwrap();

        // Check that the TODO update is formatted correctly
        assert!(formatted.contains("📝 TODO Update:"));
        assert!(formatted.contains("🔄  [1] Analyzing current usage"));
        assert!(formatted.contains("⏳  [2] Move file to new location"));
        assert!(formatted.contains("✅  [3] Update imports"));
        assert!(formatted.contains("Summary: 3 total | 1 completed | 1 in progress | 1 pending"));
    }

    #[test]
    fn test_process_todo_with_priority() {
        let mut processor = ClaudeCodeLogProcessor::new();
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "TodoWrite",
                    "input": {
                        "todos": [
                            {"content": "High priority task", "status": "pending", "activeForm": "Working on high priority", "priority": "high"},
                            {"content": "Low priority task", "status": "completed", "activeForm": "Completed low priority", "priority": "low"}
                        ]
                    }
                }]
            }
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        let formatted = result.unwrap();

        assert!(formatted.contains("⏳ 🔴 [1]")); // pending with high priority
        assert!(formatted.contains("✅ 🟢 [2]")); // completed with low priority
    }

    #[test]
    fn test_user_message_with_todo_result() {
        let mut processor = ClaudeCodeLogProcessor::new();
        let json = r#"{
            "type": "user",
            "message": {
                "role": "user",
                "content": [{"tool_use_id": "toolu_123", "type": "tool_result", "content": "Todos have been modified successfully"}]
            },
            "toolUseResult": {
                "oldTodos": [],
                "newTodos": [
                    {"content": "Task 1", "status": "pending", "activeForm": "Working on Task 1"},
                    {"content": "Task 2", "status": "in_progress", "activeForm": "Processing Task 2"}
                ]
            }
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "📝 TodoWrite result - 2 todos");
    }

    #[test]
    fn test_assistant_message_formats() {
        let mut processor = ClaudeCodeLogProcessor::new();

        // Test text message
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "Hello, world!"}]
            }
        }"#;
        let result = processor.process_line(json);
        assert_eq!(result, Some("🤖 Hello, world!".to_string()));

        // Test tool use messages
        let bash_json = r#"{
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "Bash",
                    "input": {"command": "cargo test"}
                }]
            }
        }"#;
        let result = processor.process_line(bash_json);
        assert_eq!(result, Some("🖥️ Running: cargo test".to_string()));
    }

    #[test]
    fn test_result_message() {
        let mut processor = ClaudeCodeLogProcessor::new();
        let json = r#"{
            "type": "result",
            "subtype": "success",
            "cost_usd": 0.15,
            "duration_ms": 45000,
            "result": "Task completed successfully"
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        let formatted = result.unwrap();

        assert!(formatted.contains("✅ Task Result: success"));
        assert!(formatted.contains("💰 Cost: $0.15"));
        assert!(formatted.contains("⏱️ Duration: 45 seconds"));

        // Check final result storage
        let final_result = processor.get_final_result();
        assert!(final_result.is_some());
        let task_result = final_result.unwrap();
        assert!(task_result.success);
        assert_eq!(task_result.cost_usd, Some(0.15));
    }

    #[test]
    fn test_format_duration() {
        let processor = ClaudeCodeLogProcessor::new();

        assert_eq!(processor.format_duration(0), "0 seconds");
        assert_eq!(processor.format_duration(1000), "1 second");
        assert_eq!(processor.format_duration(60000), "1 minute");
        assert_eq!(
            processor.format_duration(3661000),
            "1 hour, 1 minute, 1 second"
        );
    }

    #[test]
    fn test_summary_message() {
        let mut processor = ClaudeCodeLogProcessor::new();
        let json = r#"{
            "type": "summary",
            "summary": "Refactoring completed: Renamed XdgDirectories to TskConfig"
        }"#;

        let result = processor.process_line(json);
        assert_eq!(
            result,
            Some(
                "✅ Task Complete: Refactoring completed: Renamed XdgDirectories to TskConfig"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_empty_line_processing() {
        let mut processor = ClaudeCodeLogProcessor::new();

        assert!(processor.process_line("").is_none());
        assert!(processor.process_line("   ").is_none());
    }

    #[test]
    fn test_non_json_before_json_mode() {
        let mut processor = ClaudeCodeLogProcessor::new();

        // Before JSON mode is active, non-JSON lines should be passed through as-is
        let result = processor.process_line("Error: Claude Code is misconfigured");
        assert_eq!(
            result,
            Some("Error: Claude Code is misconfigured".to_string())
        );

        let result = processor.process_line("Please run 'claude login' to authenticate");
        assert_eq!(
            result,
            Some("Please run 'claude login' to authenticate".to_string())
        );

        // Now process a valid JSON line to activate JSON mode
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello"}]}}"#;
        let result = processor.process_line(json);
        assert_eq!(result, Some("🤖 Hello".to_string()));

        // After JSON mode is active, non-JSON lines should show parsing error
        let result = processor.process_line("This is not JSON");
        assert_eq!(result, Some("‼️ parsing error".to_string()));
    }

    #[test]
    fn test_json_mode_transition() {
        let mut processor = ClaudeCodeLogProcessor::new();

        // Initially not in JSON mode
        assert!(!processor.json_mode_active);

        // Non-JSON lines pass through
        processor.process_line("Configuration warning: API key missing");
        assert!(!processor.json_mode_active);

        // First valid JSON activates JSON mode
        let json = r#"{"type": "summary", "summary": "Task started"}"#;
        processor.process_line(json);
        assert!(processor.json_mode_active);

        // Subsequent non-JSON lines show error
        let result = processor.process_line("random text");
        assert_eq!(result, Some("‼️ parsing error".to_string()));
    }

    #[test]
    fn test_get_full_log() {
        let mut processor = ClaudeCodeLogProcessor::new();

        processor.process_line("Line 1");
        processor.process_line("Line 2");
        processor.process_line("Line 3");

        let full_log = processor.get_full_log();
        assert_eq!(full_log, "Line 1\nLine 2\nLine 3");
    }
}
