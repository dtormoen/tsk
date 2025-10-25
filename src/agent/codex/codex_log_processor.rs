use async_trait::async_trait;
use serde::Deserialize;

use crate::agent::{LogProcessor, TaskResult};

/// Represents an event from Codex's JSON output
#[derive(Debug, Deserialize)]
struct CodexEvent {
    #[serde(rename = "type")]
    event_type: String,
    item: Option<ItemData>,
    usage: Option<UsageData>,
    error: Option<ErrorData>,
}

/// Item data structure from Codex events
#[derive(Debug, Deserialize)]
struct ItemData {
    #[serde(rename = "type")]
    item_type: String,
    command: Option<String>,
    text: Option<String>,
    aggregated_output: Option<String>,
    exit_code: Option<i32>,
    changes: Option<Vec<FileChange>>,
}

/// File change information
#[derive(Debug, Deserialize)]
struct FileChange {
    path: String,
}

/// Usage information from Codex turn.completed events
#[derive(Debug, Deserialize)]
struct UsageData {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
}

/// Error information from Codex error events
#[derive(Debug, Deserialize)]
struct ErrorData {
    message: String,
}

/// Codex-specific log processor that parses and formats JSON output
///
/// This processor provides rich output including:
/// - Command execution with output previews
/// - Agent messages and reasoning
/// - File changes and tool invocations
/// - TODO list updates
/// - Turn completion with token usage and cost
///
/// The processor handles non-JSON output gracefully:
/// - Initially prints non-JSON lines as-is (for misconfiguration messages)
/// - Switches to JSON-only mode after the first valid JSON line
pub struct CodexLogProcessor {
    full_log: Vec<String>,
    final_result: Option<TaskResult>,
    json_mode_active: bool,
    /// Track whether the previous line was a parsing error to avoid duplicate error messages
    last_line_was_parse_error: bool,
    /// Optional task name for prefixing log lines
    task_name: Option<String>,
}

impl CodexLogProcessor {
    /// Creates a new CodexLogProcessor
    pub fn new(task_name: Option<String>) -> Self {
        Self {
            full_log: Vec::new(),
            final_result: None,
            json_mode_active: false,
            last_line_was_parse_error: false,
            task_name,
        }
    }

    /// Creates a prefix for log lines in the format: <emoji> [<task-name>][codex]:
    fn create_prefix(&self, emoji: &str) -> String {
        let mut prefix = emoji.to_string();

        if let Some(task_name) = &self.task_name {
            prefix.push_str(&format!(" [{}]", task_name));
        }

        prefix.push_str("[codex]: ");
        prefix
    }

    /// Simplifies a command by removing bash -lc wrapper boilerplate
    fn simplify_command(&self, command: &str) -> String {
        // Handle: bash -lc 'actual command'
        if let Some(inner) = command.strip_prefix("bash -lc '")
            && let Some(cmd) = inner.strip_suffix('\'')
        {
            return cmd.to_string();
        }

        // Handle: bash -lc "actual command"
        if let Some(inner) = command.strip_prefix("bash -lc \"")
            && let Some(cmd) = inner.strip_suffix('"')
        {
            return cmd.to_string();
        }

        // Handle: bash -lc command (no quotes)
        if let Some(inner) = command.strip_prefix("bash -lc ") {
            return inner.to_string();
        }

        // No match, return original
        command.to_string()
    }

    /// Formats a Codex event based on its type
    fn format_event(&mut self, event: CodexEvent) -> Option<String> {
        match event.event_type.as_str() {
            "thread.started" => None, // Filter out noise
            "turn.started" => None,   // Filter out noise
            "turn.completed" => self.format_turn_completed(event),
            "turn.failed" => self.format_turn_failed(event),
            "item.started" => self.format_item_started(event),
            "item.updated" => None, // Redundant with completed
            "item.completed" => self.format_item_completed(event),
            "error" => self.format_error(event),
            _ => {
                // Unknown event type - log for debugging but don't show
                None
            }
        }
    }

    /// Formats a turn.completed event with usage statistics
    fn format_turn_completed(&mut self, event: CodexEvent) -> Option<String> {
        if let Some(usage) = event.usage {
            self.final_result = Some(TaskResult {
                success: true,
                message: "Task completed successfully".to_string(),
                cost_usd: None,
                duration_ms: None,
            });

            let input = usage.input_tokens.unwrap_or(0);
            let output = usage.output_tokens.unwrap_or(0);
            let cached = usage.cached_input_tokens.unwrap_or(0);

            Some(format!(
                "{}Task completed - {} input tokens, {} output tokens, {} cached tokens",
                self.create_prefix("📊"),
                input,
                output,
                cached
            ))
        } else {
            Some(format!("{}Task completed", self.create_prefix("📊")))
        }
    }

    /// Formats a turn.failed event with error details
    fn format_turn_failed(&mut self, event: CodexEvent) -> Option<String> {
        let error_msg = event
            .error
            .as_ref()
            .map(|e| e.message.as_str())
            .unwrap_or("Unknown error");

        self.final_result = Some(TaskResult {
            success: false,
            message: format!("Turn failed: {}", error_msg),
            cost_usd: None,
            duration_ms: None,
        });

        Some(format!(
            "{}Turn failed: {}",
            self.create_prefix("❌"),
            error_msg
        ))
    }

    /// Formats an item.started event with context based on item type
    fn format_item_started(&mut self, event: CodexEvent) -> Option<String> {
        if let Some(item) = event.item {
            match item.item_type.as_str() {
                "command_execution" => {
                    let cmd = item.command.as_deref().unwrap_or("unknown");
                    let simplified_cmd = self.simplify_command(cmd);
                    Some(format!(
                        "{}Running: {}",
                        self.create_prefix("🖥️"),
                        simplified_cmd
                    ))
                }
                "agent_message" => None, // Wait for completion
                "reasoning" => Some(format!("{}Reasoning...", self.create_prefix("🧠"))),
                "file_change" => Some(format!("{}Modifying file...", self.create_prefix("📝"))),
                "mcp_tool_call" => Some(format!("{}Calling tool...", self.create_prefix("🔧"))),
                "web_search" => Some(format!("{}Searching web...", self.create_prefix("🌐"))),
                "todo_list" => None, // Wait for completion
                _ => {
                    // Unknown item type - show generic message
                    Some(format!(
                        "{}{}: started",
                        self.create_prefix("🔧"),
                        item.item_type
                    ))
                }
            }
        } else {
            None
        }
    }

    /// Formats an item.completed event with result summary
    fn format_item_completed(&mut self, event: CodexEvent) -> Option<String> {
        if let Some(item) = event.item {
            match item.item_type.as_str() {
                "command_execution" => {
                    let exit_code = item.exit_code.unwrap_or(-1);
                    let mut result_msg = String::new();

                    // Check if this is a test command with results
                    if let Some(stdout) = &item.aggregated_output {
                        if stdout.contains("test result: ok") {
                            result_msg = " - Tests passed ✅".to_string();
                        } else if stdout.contains("test result: FAILED") {
                            result_msg = " - Tests failed ❌".to_string();
                        }
                    }

                    // Only show output preview for errors (non-zero exit) or if we didn't find test results
                    if exit_code != 0
                        && result_msg.is_empty()
                        && let Some(stdout) = &item.aggregated_output
                    {
                        let preview = stdout.lines().next().unwrap_or("").trim();
                        if !preview.is_empty() {
                            if preview.len() > 60 {
                                result_msg = format!(" - {}...", &preview[..60]);
                            } else {
                                result_msg = format!(" - {}", preview);
                            }
                        }
                    }

                    Some(format!(
                        "{}Command completed (exit: {}){}",
                        self.create_prefix("🖥️"),
                        exit_code,
                        result_msg
                    ))
                }
                "agent_message" => {
                    if let Some(text) = item.text {
                        if !text.trim().is_empty() {
                            Some(format!("{}{}", self.create_prefix("🤖"), text.trim()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                "reasoning" => {
                    if let Some(text) = item.text {
                        // Show snippet of reasoning (first line, max 80 chars)
                        let snippet = text.lines().next().unwrap_or("").trim();
                        if snippet.len() > 80 {
                            Some(format!("{}{}...", self.create_prefix("🧠"), &snippet[..77]))
                        } else if !snippet.is_empty() {
                            Some(format!("{}{}", self.create_prefix("🧠"), snippet))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                "file_change" => {
                    if let Some(changes) = &item.changes {
                        // Extract filenames from paths (remove /workspace prefix if present)
                        let filenames: Vec<String> = changes
                            .iter()
                            .map(|c| {
                                c.path
                                    .strip_prefix("/workspace/")
                                    .unwrap_or(&c.path)
                                    .to_string()
                            })
                            .collect();

                        if filenames.is_empty() {
                            Some(format!("{}File modified", self.create_prefix("✅")))
                        } else if filenames.len() == 1 {
                            Some(format!(
                                "{}Modified: {}",
                                self.create_prefix("✅"),
                                filenames[0]
                            ))
                        } else {
                            // Multiple files - show count and first file
                            Some(format!(
                                "{}Modified {} files: {}, ...",
                                self.create_prefix("✅"),
                                filenames.len(),
                                filenames[0]
                            ))
                        }
                    } else {
                        Some(format!("{}File modified", self.create_prefix("✅")))
                    }
                }
                "mcp_tool_call" => Some(format!("{}Tool completed", self.create_prefix("🔧"))),
                "web_search" => Some(format!("{}Search completed", self.create_prefix("🌐"))),
                "todo_list" => {
                    if let Some(text) = item.text {
                        // Extract first TODO item as summary
                        let summary = text
                            .lines()
                            .find(|line| !line.trim().is_empty())
                            .unwrap_or("TODO updated");
                        Some(format!(
                            "{}TODO: {}",
                            self.create_prefix("📋"),
                            summary.trim()
                        ))
                    } else {
                        Some(format!("{}TODO updated", self.create_prefix("📋")))
                    }
                }
                _ => {
                    // Unknown item type - show generic completion
                    Some(format!(
                        "{}{}: completed",
                        self.create_prefix("🔧"),
                        item.item_type
                    ))
                }
            }
        } else {
            None
        }
    }

    /// Formats an error event
    fn format_error(&mut self, event: CodexEvent) -> Option<String> {
        if let Some(error) = event.error {
            Some(format!(
                "{}Error: {}",
                self.create_prefix("❌"),
                error.message
            ))
        } else {
            Some(format!("{}Error occurred", self.create_prefix("❌")))
        }
    }
}

#[async_trait]
impl LogProcessor for CodexLogProcessor {
    fn process_line(&mut self, line: &str) -> Option<String> {
        // Store the raw line for the full log
        self.full_log.push(line.to_string());

        // Skip empty lines - don't reset error tracking
        if line.trim().is_empty() {
            return None;
        }

        // Try to parse as JSON
        match serde_json::from_str::<CodexEvent>(line) {
            Ok(event) => {
                // Successfully parsed JSON - activate JSON mode if not already active
                if !self.json_mode_active {
                    self.json_mode_active = true;
                }
                // Reset the error flag on successful parse
                self.last_line_was_parse_error = false;
                self.format_event(event)
            }
            Err(_) => {
                if self.json_mode_active {
                    // Check if we should suppress this error
                    if self.last_line_was_parse_error {
                        // Suppress duplicate parsing error
                        None
                    } else {
                        // Show first parsing error in sequence
                        self.last_line_was_parse_error = true;
                        Some(format!("{}parsing error", self.create_prefix("‼️")))
                    }
                } else {
                    // Before JSON mode is active, pass through non-JSON lines as-is
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
    fn test_thread_and_turn_events() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        let thread_started = r#"{"type":"thread.started","thread_id":"test-123"}"#;
        assert_eq!(processor.process_line(thread_started), None);

        let turn_started = r#"{"type":"turn.started"}"#;
        assert_eq!(processor.process_line(turn_started), None);
    }

    #[test]
    fn test_command_execution_events() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        let started = r#"{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"ls -la","status":"in_progress"}}"#;
        let output = processor.process_line(started).unwrap();
        assert!(output.contains("🖥️ [test-task][codex]: Running: ls -la"));

        let completed = r#"{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"ls -la","aggregated_output":"file1.txt\nfile2.txt","exit_code":0,"status":"completed"}}"#;
        let output = processor.process_line(completed).unwrap();
        assert!(output.contains("🖥️ [test-task][codex]: Command completed (exit: 0)"));
        assert!(!output.contains("file1.txt")); // Should not show output for successful commands
    }

    #[test]
    fn test_agent_message_events() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        let completed = r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"This is my response"}}"#;
        let output = processor.process_line(completed).unwrap();
        assert_eq!(output, "🤖 [test-task][codex]: This is my response");
    }

    #[test]
    fn test_turn_completed_with_usage() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        let completed = r#"{"type":"turn.completed","usage":{"input_tokens":1000,"output_tokens":500,"cached_input_tokens":200}}"#;
        let output = processor.process_line(completed).unwrap();
        assert!(output.contains("📊 [test-task][codex]: Task completed"));
        assert!(output.contains("1000 input tokens"));
        assert!(output.contains("500 output tokens"));
        assert!(output.contains("200 cached tokens"));

        // Check final result was extracted
        let result = processor.get_final_result().unwrap();
        assert!(result.success);
        assert!(result.cost_usd.is_none());
    }

    #[test]
    fn test_turn_failed() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        let failed = r#"{"type":"turn.failed","error":{"message":"API request failed"}}"#;
        let output = processor.process_line(failed).unwrap();
        assert!(output.contains("❌ [test-task][codex]: Turn failed: API request failed"));

        let result = processor.get_final_result().unwrap();
        assert!(!result.success);
    }

    #[test]
    fn test_json_mode_behavior() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        // Before JSON mode, non-JSON passes through
        let result = processor.process_line("Configuration error");
        assert_eq!(result, Some("Configuration error".to_string()));
        assert!(!processor.json_mode_active);

        // First valid JSON activates JSON mode
        let json = r#"{"type":"turn.started"}"#;
        processor.process_line(json);
        assert!(processor.json_mode_active);

        // After JSON mode, non-JSON shows parsing error
        let result = processor.process_line("Not JSON");
        assert!(result.unwrap().contains("[codex]: parsing error"));
    }

    #[test]
    fn test_parse_error_deduplication() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        // Activate JSON mode
        processor.process_line(r#"{"type":"turn.started"}"#);

        // First error shown
        let result = processor.process_line("Bad line 1");
        assert!(result.unwrap().contains("[codex]: parsing error"));

        // Subsequent errors suppressed
        assert_eq!(processor.process_line("Bad line 2"), None);
        assert_eq!(processor.process_line("Bad line 3"), None);

        // Valid JSON resets
        processor.process_line(r#"{"type":"turn.started"}"#);

        // Next error shown again
        let result = processor.process_line("Bad line 4");
        assert!(result.unwrap().contains("[codex]: parsing error"));
    }

    #[test]
    fn test_reasoning_and_file_change() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        // Reasoning started
        let started = r#"{"type":"item.started","item":{"id":"item_1","type":"reasoning"}}"#;
        let output = processor.process_line(started).unwrap();
        assert!(output.contains("🧠 [test-task][codex]: Reasoning..."));

        // Reasoning completed with text
        let completed = r#"{"type":"item.completed","item":{"id":"item_1","type":"reasoning","text":"I need to analyze the code structure"}}"#;
        let output = processor.process_line(completed).unwrap();
        assert!(output.contains("🧠 [test-task][codex]: I need to analyze"));

        // File change with path
        let file_completed = r#"{"type":"item.completed","item":{"id":"item_2","type":"file_change","changes":[{"path":"/workspace/src/main.rs","kind":"update"}]}}"#;
        let output = processor.process_line(file_completed).unwrap();
        assert!(output.contains("✅ [test-task][codex]: Modified: src/main.rs"));

        // File change without changes array (fallback)
        let file_completed_no_changes =
            r#"{"type":"item.completed","item":{"id":"item_3","type":"file_change"}}"#;
        let output = processor.process_line(file_completed_no_changes).unwrap();
        assert!(output.contains("✅ [test-task][codex]: File modified"));
    }

    #[test]
    fn test_multiple_file_changes() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        // Multiple file changes
        let file_completed = r#"{"type":"item.completed","item":{"id":"item_1","type":"file_change","changes":[{"path":"/workspace/src/main.rs","kind":"update"},{"path":"/workspace/src/lib.rs","kind":"update"},{"path":"/workspace/Cargo.toml","kind":"update"}]}}"#;
        let output = processor.process_line(file_completed).unwrap();
        assert!(output.contains("✅ [test-task][codex]: Modified 3 files: src/main.rs, ..."));
    }

    #[test]
    fn test_empty_lines_dont_reset_error_tracking() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        // Activate JSON mode
        let json = r#"{"type":"turn.started"}"#;
        processor.process_line(json);

        // First parsing error shown
        let result = processor.process_line("Bad line 1");
        assert!(result.unwrap().contains("[codex]: parsing error"));

        // Empty line should not reset error tracking
        let result = processor.process_line("");
        assert_eq!(result, None);

        // Next parsing error should still be suppressed
        let result = processor.process_line("Bad line 2");
        assert_eq!(result, None);
    }

    #[test]
    fn test_unknown_item_types() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        let started = r#"{"type":"item.started","item":{"id":"item_1","type":"unknown_type"}}"#;
        let output = processor.process_line(started).unwrap();
        assert!(output.contains("🔧 [test-task][codex]: unknown_type: started"));

        let completed = r#"{"type":"item.completed","item":{"id":"item_1","type":"unknown_type"}}"#;
        let output = processor.process_line(completed).unwrap();
        assert!(output.contains("🔧 [test-task][codex]: unknown_type: completed"));
    }

    #[test]
    fn test_todo_list() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        let completed = r#"{"type":"item.completed","item":{"id":"item_1","type":"todo_list","text":"- Implement feature A\n- Test feature A\n- Document feature A"}}"#;
        let output = processor.process_line(completed).unwrap();
        assert!(output.contains("📋 [test-task][codex]: TODO:"));
        assert!(output.contains("Implement feature A"));
    }

    #[test]
    fn test_long_output_truncation() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        // Test command with long output - should not show output for exit 0
        let long_output = "a".repeat(100);
        let completed = format!(
            r#"{{"type":"item.completed","item":{{"id":"item_0","type":"command_execution","aggregated_output":"{}","exit_code":0}}}}"#,
            long_output
        );
        let output = processor.process_line(&completed).unwrap();
        assert!(output.contains("[codex]: Command completed (exit: 0)"));
        assert!(!output.contains("aaa")); // Should not show output for successful commands

        // Test reasoning with long text
        let long_reasoning = "b".repeat(100);
        let completed = format!(
            r#"{{"type":"item.completed","item":{{"id":"item_1","type":"reasoning","text":"{}"}}}}"#,
            long_reasoning
        );
        let output = processor.process_line(&completed).unwrap();
        assert!(output.contains("..."));
        assert!(output.len() < long_reasoning.len() + 50); // Should be truncated
    }

    #[test]
    fn test_empty_agent_message() {
        let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

        let completed = r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"   "}}"#;
        let output = processor.process_line(completed);
        assert_eq!(output, None); // Empty messages should be filtered
    }

    #[test]
    fn test_command_simplification() {
        let mut processor = CodexLogProcessor::new(Some("test".to_string()));

        // Test single-quoted command
        let event = r#"{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"bash -lc 'ls -la'","status":"in_progress"}}"#;
        let output = processor.process_line(event).unwrap();
        assert!(output.contains("Running: ls -la"));
        assert!(!output.contains("bash -lc"));

        // Test double-quoted command
        let event = r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"bash -lc \"echo hello\"","status":"in_progress"}}"#;
        let output = processor.process_line(event).unwrap();
        assert!(output.contains("Running: echo hello"));

        // Test nested quotes
        let event = r#"{"type":"item.started","item":{"id":"item_2","type":"command_execution","command":"bash -lc 'rg \"fn main\" src'","status":"in_progress"}}"#;
        let output = processor.process_line(event).unwrap();
        assert!(output.contains("Running: rg \"fn main\" src"));

        // Test command without quotes
        let event = r#"{"type":"item.started","item":{"id":"item_3","type":"command_execution","command":"bash -lc ls","status":"in_progress"}}"#;
        let output = processor.process_line(event).unwrap();
        assert!(output.contains("Running: ls"));
        assert!(!output.contains("bash -lc"));

        // Test multi-word command without quotes
        let event = r#"{"type":"item.started","item":{"id":"item_4","type":"command_execution","command":"bash -lc cat file.txt","status":"in_progress"}}"#;
        let output = processor.process_line(event).unwrap();
        assert!(output.contains("Running: cat file.txt"));
        assert!(!output.contains("bash -lc"));
    }

    #[test]
    fn test_verbose_output_filtering() {
        let mut processor = CodexLogProcessor::new(Some("test".to_string()));

        // Success with routine output - should hide output
        let event = r#"{"type":"item.completed","item":{"type":"command_execution","aggregated_output":"file1.txt\nfile2.txt","exit_code":0}}"#;
        let output = processor.process_line(event).unwrap();
        assert!(output.contains("Command completed (exit: 0)"));
        assert!(!output.contains("file1.txt"));

        // Test results - should show test status
        let event = r#"{"type":"item.completed","item":{"type":"command_execution","aggregated_output":"test result: ok. 13 passed","exit_code":0}}"#;
        let output = processor.process_line(event).unwrap();
        assert!(output.contains("Tests passed ✅"));

        // Error - should show output
        let event = r#"{"type":"item.completed","item":{"type":"command_execution","aggregated_output":"error: file not found","exit_code":1}}"#;
        let output = processor.process_line(event).unwrap();
        assert!(output.contains("error: file not found"));
    }
}
