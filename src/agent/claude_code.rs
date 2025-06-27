use super::{Agent, LogProcessor};
use crate::context::file_system::FileSystemOperations;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

/// Claude Code AI agent implementation
pub struct ClaudeCodeAgent;

impl ClaudeCodeAgent {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeCodeAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Agent for ClaudeCodeAgent {
    fn build_command(&self, instruction_path: &str) -> Vec<String> {
        // Get just the filename from the instruction path
        let filename = Path::new(instruction_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("instructions.md");

        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "cat /instructions/{} | claude -p --verbose --output-format stream-json --dangerously-skip-permissions",
                filename
            ),
        ]
    }

    fn volumes(&self) -> Vec<(String, String, String)> {
        // Get the home directory path for mounting ~/.claude and ~/.claude.json
        let home_dir = std::env::var("HOME").unwrap_or_else(|_| "/home/agent".to_string());

        vec![
            // Claude config directory
            (
                format!("{home_dir}/.claude"),
                "/home/agent/.claude".to_string(),
                "".to_string(),
            ),
            // Claude config file
            (
                format!("{home_dir}/.claude.json"),
                "/home/agent/.claude.json".to_string(),
                "".to_string(),
            ),
        ]
    }

    fn environment(&self) -> Vec<(String, String)> {
        vec![
            ("HOME".to_string(), "/home/agent".to_string()),
            ("USER".to_string(), "agent".to_string()),
        ]
    }

    fn create_log_processor(
        &self,
        file_system: Arc<dyn FileSystemOperations>,
    ) -> Box<dyn LogProcessor> {
        Box::new(ClaudeCodeLogProcessor::new(file_system))
    }

    fn name(&self) -> &str {
        "claude-code"
    }

    async fn validate(&self) -> Result<(), String> {
        // Check if ~/.claude.json exists
        let home_dir = std::env::var("HOME").map_err(|_| "HOME environment variable not set")?;
        let claude_config = Path::new(&home_dir).join(".claude.json");

        if !claude_config.exists() {
            return Err(format!(
                "Claude configuration not found at {}. Please run 'claude login' first.",
                claude_config.display()
            ));
        }

        Ok(())
    }

    async fn warmup(&self) -> Result<(), String> {
        // Skip warmup in test environments
        if cfg!(test) {
            return Ok(());
        }

        println!("Running Claude Code warmup steps...");

        // Step 1: Force Claude CLI to refresh token
        let output = Command::new("claude")
            .args(["-p", "--model", "sonnet", "say hi and nothing else"])
            .output()
            .map_err(|e| format!("Failed to run Claude CLI: {e}"))?;

        if !output.status.success() {
            return Err(format!(
                "Claude CLI failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        // Step 2: Export OAuth token on macOS only
        if std::env::consts::OS == "macos" {
            let user = std::env::var("USER").map_err(|_| "USER environment variable not set")?;

            let output = Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "security find-generic-password -a {user} -w -s 'Claude Code-credentials' > ~/.claude/.credentials.json"
                ))
                .output()
                .map_err(|e| format!("Failed to export OAuth token: {e}"))?;

            if !output.status.success() {
                // This might fail if the keychain item doesn't exist yet
                eprintln!("Warning: Could not export OAuth token from keychain");
            }
        }

        println!("Claude Code warmup completed successfully");
        Ok(())
    }
}

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
    tool_use_result: Option<ToolUseResult>,
    summary: Option<String>,
}

/// Tool use result from user messages
#[derive(Debug, Deserialize, Serialize)]
struct ToolUseResult {
    stdout: Option<String>,
    stderr: Option<String>,
    is_error: Option<bool>,
    error: Option<String>,
    filenames: Option<Vec<String>>,
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
    id: String,
    content: String,
    status: String,
    priority: String,
}

/// Result of a task execution from Claude Code
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub success: bool,
    pub message: String,
    #[allow(dead_code)] // Available for future use
    pub cost_usd: Option<f64>,
    #[allow(dead_code)] // Available for future use
    pub duration_ms: Option<u64>,
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
struct ClaudeCodeLogProcessor {
    full_log: Vec<String>,
    final_result: Option<TaskResult>,
    file_system: Arc<dyn FileSystemOperations>,
}

impl ClaudeCodeLogProcessor {
    /// Creates a new ClaudeCodeLogProcessor with the given file system
    fn new(file_system: Arc<dyn FileSystemOperations>) -> Self {
        Self {
            full_log: Vec::new(),
            final_result: None,
            file_system,
        }
    }

    /// Formats a Claude message based on its type
    fn format_message(&mut self, msg: ClaudeMessage) -> Option<String> {
        match msg.message_type.as_str() {
            "assistant" => self.format_assistant_message(msg),
            "user" => self.format_user_message(&msg),
            "result" => self.format_result_message(msg),
            "summary" => {
                // Show summary messages
                if let Some(summary_text) = msg.summary {
                    Some(format!("📋 Summary: {summary_text}"))
                } else {
                    Some("📋 [summary]".to_string())
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
        if let Some(tool_result) = &msg.tool_use_result {
            let mut output = String::new();

            // Check for specific tool results
            if let Some(filenames) = &tool_result.filenames {
                let count = filenames.len();
                output.push_str(&format!(
                    "👤 Tool result: Found {count} file{}",
                    if count == 1 { "" } else { "s" }
                ));
            } else if let Some(stdout) = &tool_result.stdout {
                if stdout.contains("test result: ok") {
                    output.push_str("👤 Tool result: Tests passed ✅");
                } else if stdout.contains("test result: FAILED") {
                    output.push_str("👤 Tool result: Tests failed ❌");
                } else if stdout.trim().is_empty() {
                    output.push_str("👤 Tool result: Command completed");
                } else {
                    // For other outputs, show a brief summary
                    let first_line = stdout.lines().next().unwrap_or("").trim();
                    if first_line.len() > 50 {
                        output.push_str(&format!("👤 Tool result: {}...", &first_line[..50]));
                    } else {
                        output.push_str(&format!("👤 Tool result: {first_line}"));
                    }
                }
            } else if let Some(error) = &tool_result.error {
                output.push_str(&format!("👤 Tool error: {error}"));
            } else if tool_result.is_error == Some(true) {
                output.push_str("👤 Tool result: Error occurred");
            } else {
                output.push_str("👤 Tool result: Completed");
            }

            Some(output)
        } else {
            // Check if this is a regular user message with content
            if let Some(message) = &msg.message {
                if let Some(Value::String(text)) = &message.content {
                    // Show brief summary of user message
                    let first_line = text.lines().next().unwrap_or("").trim();
                    if first_line.starts_with("# ") {
                        Some(format!("👤 User: {}", first_line.trim_start_matches("# ")))
                    } else if first_line.len() > 60 {
                        Some(format!("👤 User: {}...", &first_line[..60]))
                    } else if !first_line.is_empty() {
                        Some(format!("👤 User: {first_line}"))
                    } else {
                        Some("👤 [user]".to_string())
                    }
                } else {
                    Some("👤 [user]".to_string())
                }
            } else {
                Some("👤 [user]".to_string())
            }
        }
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
                            if let Some(tool_name) = item.get("name").and_then(|n| n.as_str()) {
                                match tool_name {
                                    "TodoWrite" => {
                                        if let Some(input) = item.get("input") {
                                            if let Some(todos) = input.get("todos") {
                                                if let Ok(todo_items) =
                                                    serde_json::from_value::<Vec<TodoItem>>(
                                                        todos.clone(),
                                                    )
                                                {
                                                    output.push_str(
                                                        &self.format_todo_update(&todo_items),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    "Read" | "LS" | "NotebookRead" => {
                                        // Skip file reading operations as requested
                                    }
                                    "Edit" | "MultiEdit" => {
                                        if let Some(input) = item.get("input") {
                                            if let Some(file_path) =
                                                input.get("file_path").and_then(|f| f.as_str())
                                            {
                                                let file_name = file_path
                                                    .rsplit('/')
                                                    .next()
                                                    .unwrap_or(file_path);
                                                if tool_name == "MultiEdit" {
                                                    if let Some(edits) = input
                                                        .get("edits")
                                                        .and_then(|e| e.as_array())
                                                    {
                                                        output.push_str(&format!(
                                                            "🔧 Editing {file_name} ({} changes)\n",
                                                            edits.len()
                                                        ));
                                                    } else {
                                                        output.push_str(&format!(
                                                            "🔧 Editing {file_name}\n"
                                                        ));
                                                    }
                                                } else {
                                                    output.push_str(&format!(
                                                        "🔧 Editing {file_name}\n"
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                    "Bash" => {
                                        if let Some(input) = item.get("input") {
                                            if let Some(cmd) =
                                                input.get("command").and_then(|c| c.as_str())
                                            {
                                                let cmd_preview = if cmd.len() > 60 {
                                                    format!("{}...", &cmd[..60])
                                                } else {
                                                    cmd.to_string()
                                                };
                                                output.push_str(&format!(
                                                    "🖥️ Running: {cmd_preview}\n"
                                                ));
                                            }
                                        }
                                    }
                                    "Write" => {
                                        if let Some(input) = item.get("input") {
                                            if let Some(file_path) =
                                                input.get("file_path").and_then(|f| f.as_str())
                                            {
                                                let file_name = file_path
                                                    .rsplit('/')
                                                    .next()
                                                    .unwrap_or(file_path);
                                                output
                                                    .push_str(&format!("📝 Writing {file_name}\n"));
                                            }
                                        }
                                    }
                                    "Grep" => {
                                        if let Some(input) = item.get("input") {
                                            if let Some(pattern) =
                                                input.get("pattern").and_then(|p| p.as_str())
                                            {
                                                output.push_str(&format!(
                                                    "🔍 Searching for: {pattern}\n"
                                                ));
                                            }
                                        }
                                    }
                                    "WebSearch" => {
                                        if let Some(input) = item.get("input") {
                                            if let Some(query) =
                                                input.get("query").and_then(|q| q.as_str())
                                            {
                                                output
                                                    .push_str(&format!("🌐 Web search: {query}\n"));
                                            }
                                        }
                                    }
                                    _ => {
                                        // Other tools
                                        output.push_str(&format!("🔧 Using {tool_name}\n"));
                                    }
                                }
                            }

                            // Process regular text content
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                if !text.trim().is_empty() {
                                    if !output.is_empty() {
                                        output.push('\n');
                                    }
                                    output.push_str(&format!("🤖 {text}"));
                                }
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

    /// Formats a todo list update with status indicators and summary
    fn format_todo_update(&self, todos: &[TodoItem]) -> String {
        let mut output = String::new();
        output.push_str("📝 TODO Update:\n");
        output.push_str(&"─".repeat(60));
        output.push('\n');

        // Display todos in their original order with status emoji
        for todo in todos {
            let status_emoji = match todo.status.as_str() {
                "in_progress" => "🔄",
                "pending" => "⏳",
                "completed" => "✅",
                _ => "⏳", // Default to pending for unknown statuses
            };

            output.push_str(&format!(
                "{} {} [{}] {}\n",
                status_emoji,
                self.get_priority_emoji(&todo.priority),
                todo.id,
                todo.content
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

    /// Formats duration in milliseconds to a human-readable string.
    ///
    /// # Examples
    /// - 1500 ms -> "1 second"
    /// - 130000 ms -> "2 minutes, 10 seconds"
    /// - 3661000 ms -> "1 hour, 1 minute, 1 second"
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
        // These are rough estimates and should be updated based on actual pricing
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
            Ok(msg) => self.format_message(msg),
            Err(_) => {
                // If it's not JSON, return a parsing error indicator
                Some("‼️ parsing error".to_string())
            }
        }
    }

    fn get_full_log(&self) -> String {
        self.full_log.join("\n")
    }

    async fn save_full_log(&self, path: &Path) -> Result<(), String> {
        let content = self.full_log.join("\n");
        self.file_system
            .write_file(path, &content)
            .await
            .map_err(|e| format!("Failed to save log file: {e}"))
    }

    fn get_final_result(&self) -> Option<&TaskResult> {
        self.final_result.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::file_system::tests::MockFileSystem;

    #[test]
    fn test_process_assistant_message() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "Hello, world!"}]
            }
        }"#;

        let result = processor.process_line(json);
        assert_eq!(result, Some("🤖 Hello, world!".to_string()));
    }

    #[test]
    fn test_process_result_message() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "result",
            "subtype": "success",
            "cost_usd": 0.123,
            "result": "Task completed successfully with all tests passing"
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        let formatted = result.unwrap();
        assert!(formatted.contains("✅ Task Result: success"));
        assert!(formatted.contains("💰 Cost: $0.12"));

        // Check that the result was parsed correctly
        let final_result = processor.get_final_result();
        assert!(final_result.is_some());
        let task_result = final_result.unwrap();
        assert_eq!(task_result.success, true);
        assert_eq!(
            task_result.message,
            "Task completed successfully with all tests passing"
        );
        assert_eq!(task_result.cost_usd, Some(0.123));
    }

    #[test]
    fn test_process_result_message_failure() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "result",
            "subtype": "error",
            "is_error": true,
            "result": "Task failed due to compilation errors",
            "duration_ms": 5000
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        let formatted = result.unwrap();
        assert!(formatted.contains("❌ Task Result: error"));
        assert!(formatted.contains("⏱️ Duration: 5 seconds"));

        // Check that the failure was parsed correctly
        let final_result = processor.get_final_result();
        assert!(final_result.is_some());
        let task_result = final_result.unwrap();
        assert_eq!(task_result.success, false);
        assert_eq!(task_result.message, "Task failed due to compilation errors");
        assert_eq!(task_result.duration_ms, Some(5000));
    }

    #[test]
    fn test_process_non_json() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let line = "This is not JSON";

        let result = processor.process_line(line);
        assert_eq!(result, Some("‼️ parsing error".to_string()));
    }

    #[test]
    fn test_process_other_message_types() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);

        // Test a message with an unknown type - tool_use
        let json = r#"{"type": "tool_use"}"#;
        let result = processor.process_line(json);
        assert_eq!(result, Some("📋 [tool_use]".to_string()));

        // Test another unknown type - system
        let json = r#"{"type": "system"}"#;
        let result = processor.process_line(json);
        assert_eq!(result, Some("📋 [system]".to_string()));

        // Test with more complete message structure
        let json = r#"{"type": "thinking", "message": {"content": "Processing..."}}"#;
        let result = processor.process_line(json);
        assert_eq!(result, Some("📋 [thinking]".to_string()));
    }

    #[test]
    fn test_process_todo_update() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "assistant",
            "message": {
                "id": "msg_01715dTbzrJ49yvb5Mp68sQa",
                "type": "message",
                "role": "assistant",
                "model": "claude-opus-4-20250514",
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_013pfL2AAyzkXVLeuGBrD2Z1",
                    "name": "TodoWrite",
                    "input": {
                        "todos": [
                            {"id": "1", "content": "Analyze existing MockDockerClient implementations", "status": "pending", "priority": "high"},
                            {"id": "2", "content": "Create test_utils module structure", "status": "pending", "priority": "high"},
                            {"id": "3", "content": "Implement NoOpDockerClient", "status": "completed", "priority": "high"},
                            {"id": "4", "content": "Run tests and fix any issues", "status": "in_progress", "priority": "medium"}
                        ]
                    }
                }]
            }
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        let formatted = result.unwrap();

        // Check that the TODO update header is present
        assert!(formatted.contains("📝 TODO Update:"));

        // Check that todos have status emojis in the original order
        let lines: Vec<&str> = formatted.lines().collect();
        let todo_lines: Vec<&str> = lines
            .iter()
            .filter(|line| {
                line.contains("[1]")
                    || line.contains("[2]")
                    || line.contains("[3]")
                    || line.contains("[4]")
            })
            .cloned()
            .collect();

        // Verify the order is preserved as in the input
        assert_eq!(todo_lines.len(), 4);
        assert!(todo_lines[0].starts_with("⏳ 🔴 [1]")); // pending, high priority
        assert!(todo_lines[1].starts_with("⏳ 🔴 [2]")); // pending, high priority
        assert!(todo_lines[2].starts_with("✅ 🔴 [3]")); // completed, high priority
        assert!(todo_lines[3].starts_with("🔄 🟡 [4]")); // in_progress, medium priority

        // Check that specific todo items are present
        assert!(formatted.contains("Analyze existing MockDockerClient implementations"));
        assert!(formatted.contains("Create test_utils module structure"));
        assert!(formatted.contains("Implement NoOpDockerClient"));
        assert!(formatted.contains("Run tests and fix any issues"));

        // Check priority emojis
        assert!(formatted.contains("🔴")); // high priority
        assert!(formatted.contains("🟡")); // medium priority

        // Check summary
        assert!(formatted.contains("Summary: 4 total | 1 completed | 1 in progress | 2 pending"));
    }

    #[test]
    fn test_process_todo_update_all_completed() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "TodoWrite",
                    "input": {
                        "todos": [
                            {"id": "1", "content": "Task 1", "status": "completed", "priority": "high"},
                            {"id": "2", "content": "Task 2", "status": "completed", "priority": "low"}
                        ]
                    }
                }]
            }
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        let formatted = result.unwrap();

        // Check that both todos have completed status emoji
        let lines: Vec<&str> = formatted.lines().collect();
        let todo_lines: Vec<&str> = lines
            .iter()
            .filter(|line| line.contains("[1]") || line.contains("[2]"))
            .cloned()
            .collect();

        assert_eq!(todo_lines.len(), 2);
        assert!(todo_lines[0].starts_with("✅ 🔴 [1]")); // completed, high priority
        assert!(todo_lines[1].starts_with("✅ 🟢 [2]")); // completed, low priority

        // Check summary
        assert!(formatted.contains("Summary: 2 total | 2 completed | 0 in progress | 0 pending"));
    }

    #[test]
    fn test_todo_priority_emojis() {
        let fs = Arc::new(MockFileSystem::new());
        let processor = ClaudeCodeLogProcessor::new(fs);
        assert_eq!(processor.get_priority_emoji("high"), "🔴");
        assert_eq!(processor.get_priority_emoji("medium"), "🟡");
        assert_eq!(processor.get_priority_emoji("low"), "🟢");
        assert_eq!(processor.get_priority_emoji("unknown"), "⚪");
    }

    #[test]
    fn test_process_result_message_with_long_duration() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "result",
            "subtype": "success",
            "duration_ms": 130000,
            "result": "Task completed after processing"
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        let formatted = result.unwrap();
        assert!(formatted.contains("✅ Task Result: success"));
        assert!(formatted.contains("⏱️ Duration: 2 minutes, 10 seconds"));
    }

    #[test]
    fn test_format_duration() {
        let fs = Arc::new(MockFileSystem::new());
        let processor = ClaudeCodeLogProcessor::new(fs);

        // Test seconds only
        assert_eq!(processor.format_duration(0), "0 seconds");
        assert_eq!(processor.format_duration(1000), "1 second");
        assert_eq!(processor.format_duration(45000), "45 seconds");

        // Test minutes and seconds
        assert_eq!(processor.format_duration(60000), "1 minute");
        assert_eq!(processor.format_duration(61000), "1 minute, 1 second");
        assert_eq!(processor.format_duration(130000), "2 minutes, 10 seconds");
        assert_eq!(processor.format_duration(180000), "3 minutes");

        // Test hours, minutes, and seconds
        assert_eq!(processor.format_duration(3600000), "1 hour");
        assert_eq!(
            processor.format_duration(3661000),
            "1 hour, 1 minute, 1 second"
        );
        assert_eq!(
            processor.format_duration(7321000),
            "2 hours, 2 minutes, 1 second"
        );
        assert_eq!(processor.format_duration(10800000), "3 hours");

        // Test edge cases
        assert_eq!(processor.format_duration(3660000), "1 hour, 1 minute");
        assert_eq!(processor.format_duration(7200000), "2 hours");
        assert_eq!(
            processor.format_duration(86399000),
            "23 hours, 59 minutes, 59 seconds"
        );
    }

    #[test]
    fn test_todo_order_preservation() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "TodoWrite",
                    "input": {
                        "todos": [
                            {"id": "1", "content": "First task", "status": "completed", "priority": "low"},
                            {"id": "2", "content": "Second task", "status": "in_progress", "priority": "high"},
                            {"id": "3", "content": "Third task", "status": "pending", "priority": "medium"},
                            {"id": "4", "content": "Fourth task", "status": "completed", "priority": "high"},
                            {"id": "5", "content": "Fifth task", "status": "pending", "priority": "low"}
                        ]
                    }
                }]
            }
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        let formatted = result.unwrap();

        // Extract the todo lines in order
        let lines: Vec<&str> = formatted.lines().collect();
        let todo_lines: Vec<&str> = lines
            .iter()
            .filter(|line| {
                line.contains("task")
                    && (line.contains("[1]")
                        || line.contains("[2]")
                        || line.contains("[3]")
                        || line.contains("[4]")
                        || line.contains("[5]"))
            })
            .cloned()
            .collect();

        // Verify order is exactly as provided in input
        assert_eq!(todo_lines.len(), 5);
        assert!(todo_lines[0].contains("First task") && todo_lines[0].starts_with("✅"));
        assert!(todo_lines[1].contains("Second task") && todo_lines[1].starts_with("🔄"));
        assert!(todo_lines[2].contains("Third task") && todo_lines[2].starts_with("⏳"));
        assert!(todo_lines[3].contains("Fourth task") && todo_lines[3].starts_with("✅"));
        assert!(todo_lines[4].contains("Fifth task") && todo_lines[4].starts_with("⏳"));
    }

    #[test]
    fn test_user_message() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "user",
            "message": {
                "content": "User input message"
            }
        }"#;

        let result = processor.process_line(json);
        assert_eq!(result, Some("👤 User: User input message".to_string()));
    }

    #[test]
    fn test_user_message_with_tool_result() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "user",
            "toolUseResult": {
                "stdout": "test result: ok. 5 passed; 0 failed",
                "stderr": "",
                "is_error": false
            }
        }"#;

        let result = processor.process_line(json);
        assert_eq!(result, Some("👤 Tool result: Tests passed ✅".to_string()));
    }

    #[test]
    fn test_user_message_with_file_search_result() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "user",
            "toolUseResult": {
                "filenames": ["file1.rs", "file2.rs", "file3.rs"]
            }
        }"#;

        let result = processor.process_line(json);
        assert_eq!(result, Some("👤 Tool result: Found 3 files".to_string()));
    }

    #[test]
    fn test_assistant_message_with_tool_use() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "Bash",
                    "input": {
                        "command": "cargo test --all"
                    }
                }]
            }
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        assert!(result.unwrap().contains("🖥️ Running: cargo test --all"));
    }

    #[test]
    fn test_assistant_message_with_edit_tool() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "Edit",
                    "input": {
                        "file_path": "/workspace/src/main.rs"
                    }
                }]
            }
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        assert!(result.unwrap().contains("🔧 Editing main.rs"));
    }

    #[test]
    fn test_cost_calculation_from_usage() {
        let fs = Arc::new(MockFileSystem::new());
        let processor = ClaudeCodeLogProcessor::new(fs);

        let usage = Usage {
            input_tokens: Some(1000),
            output_tokens: Some(500),
            cache_creation_input_tokens: Some(0),
            cache_read_input_tokens: Some(0),
            service_tier: Some("standard".to_string()),
        };

        let cost = processor.calculate_cost_from_usage(&usage);
        assert!(cost.is_some());
        // 1000 * 0.015 / 1000 + 500 * 0.075 / 1000 = 0.015 + 0.0375 = 0.0525
        assert!((cost.unwrap() - 0.0525).abs() < 0.0001);
    }

    #[test]
    fn test_summary_message() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);
        let json = r#"{
            "type": "summary",
            "summary": "Test Summary: Running unit tests"
        }"#;

        let result = processor.process_line(json);
        assert_eq!(
            result,
            Some("📋 Summary: Test Summary: Running unit tests".to_string())
        );
    }

    #[test]
    fn test_empty_line_processing() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);

        let result = processor.process_line("");
        assert!(result.is_none());

        let result = processor.process_line("   ");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_save_full_log() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs.clone());

        // Process some lines
        processor.process_line(r#"{"type": "user"}"#);
        processor.process_line("Regular text line");
        processor.process_line(r#"{"type": "result", "subtype": "success"}"#);

        // Save the log
        let path = std::path::Path::new("/test/log.txt");
        let result = processor.save_full_log(path).await;
        assert!(result.is_ok());

        // Verify the log was saved
        let content = fs.read_file(path).await.unwrap();
        assert!(content.contains(r#"{"type": "user"}"#));
        assert!(content.contains("Regular text line"));
        assert!(content.contains(r#"{"type": "result", "subtype": "success"}"#));
    }

    #[test]
    fn test_get_full_log() {
        let fs = Arc::new(MockFileSystem::new());
        let mut processor = ClaudeCodeLogProcessor::new(fs);

        // Process some lines
        processor.process_line("Line 1");
        processor.process_line("Line 2");
        processor.process_line("Line 3");

        let full_log = processor.get_full_log();
        assert_eq!(full_log, "Line 1\nLine 2\nLine 3");
    }

    #[test]
    fn test_claude_code_agent_properties() {
        let agent = ClaudeCodeAgent::new();

        // Test name
        assert_eq!(agent.name(), "claude-code");

        // Test volumes
        let volumes = agent.volumes();
        assert_eq!(volumes.len(), 2);

        // Should mount .claude directory and .claude.json file
        let volume_paths: Vec<&str> = volumes
            .iter()
            .map(|(_, container_path, _)| container_path.as_str())
            .collect();
        assert!(volume_paths.contains(&"/home/agent/.claude"));
        assert!(volume_paths.contains(&"/home/agent/.claude.json"));

        // Test environment variables
        let env = agent.environment();
        assert_eq!(env.len(), 2);

        let env_map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(env_map.get("HOME"), Some(&"/home/agent".to_string()));
        assert_eq!(env_map.get("USER"), Some(&"agent".to_string()));
    }

    #[test]
    fn test_claude_code_agent_build_command() {
        let agent = ClaudeCodeAgent::new();

        // Test with full path
        let command = agent.build_command("/tmp/instructions.md");
        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("cat /instructions/instructions.md"));
        assert!(command[2].contains("claude -p --verbose --output-format stream-json"));

        // Test with complex path
        let command = agent.build_command("/path/to/task/instructions.txt");
        assert!(command[2].contains("cat /instructions/instructions.txt"));
    }

    #[tokio::test]
    async fn test_claude_code_agent_validate_without_config() {
        // Create a temporary HOME directory without .claude.json
        let temp_dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("HOME", temp_dir.path());
        }

        let agent = ClaudeCodeAgent::new();
        let result = agent.validate().await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Claude configuration not found")
        );
    }

    #[test]
    fn test_claude_code_agent_create_log_processor() {
        let fs = Arc::new(MockFileSystem::new());
        let agent = ClaudeCodeAgent::new();

        let log_processor = agent.create_log_processor(fs);

        // Just verify we can create a log processor
        // The actual log processor functionality is tested elsewhere
        let _ = log_processor.get_full_log();
    }
}
