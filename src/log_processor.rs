use crate::context::file_system::FileSystemOperations;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

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
}

#[derive(Debug, Deserialize, Serialize)]
struct MessageContent {
    role: Option<String>,
    content: Option<Value>,
    id: Option<String>,
    #[serde(rename = "type")]
    content_type: Option<String>,
    model: Option<String>,
    stop_reason: Option<String>,
    usage: Option<Value>,
}

pub struct LogProcessor {
    full_log: Vec<String>,
    final_result: Option<TaskResult>,
    file_system: Option<Arc<dyn FileSystemOperations>>,
}

#[derive(Debug, Clone)]
pub struct TaskResult {
    pub success: bool,
    pub message: String,
    #[allow(dead_code)] // Available for future use
    pub cost_usd: Option<f64>,
    #[allow(dead_code)] // Available for future use
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct TodoItem {
    id: String,
    content: String,
    status: String,
    priority: String,
}

impl LogProcessor {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            full_log: Vec::new(),
            final_result: None,
            file_system: None,
        }
    }

    pub fn with_file_system(file_system: Arc<dyn FileSystemOperations>) -> Self {
        Self {
            full_log: Vec::new(),
            final_result: None,
            file_system: Some(file_system),
        }
    }

    pub fn process_line(&mut self, line: &str) -> Option<String> {
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
                // If it's not JSON, just output the line as-is
                // Some(line.to_string())
                Some("‼️ parsing error".to_string())
            }
        }
    }

    fn format_message(&mut self, msg: ClaudeMessage) -> Option<String> {
        match msg.message_type.as_str() {
            "assistant" => self.format_assistant_message(msg),
            "user" => self.format_user_message(),
            "result" => self.format_result_message(msg),
            other_type => {
                // For other message types, just show a brief indicator
                Some(format!("📋 [{}]", other_type))
            }
        }
    }

    fn format_user_message(&self) -> Option<String> {
        Some("👤 [user]".to_string())
    }

    fn format_assistant_message(&self, msg: ClaudeMessage) -> Option<String> {
        if let Some(message) = msg.message {
            if let Some(content) = message.content {
                match content {
                    Value::Array(contents) => {
                        let mut output = String::new();
                        let mut has_todo_update = false;

                        for item in contents {
                            // Check for TodoWrite tool use
                            if let Some(tool_name) = item.get("name").and_then(|n| n.as_str()) {
                                if tool_name == "TodoWrite" {
                                    if let Some(input) = item.get("input") {
                                        if let Some(todos) = input.get("todos") {
                                            if let Ok(todo_items) =
                                                serde_json::from_value::<Vec<TodoItem>>(
                                                    todos.clone(),
                                                )
                                            {
                                                has_todo_update = true;
                                                output.push_str(
                                                    &self.format_todo_update(&todo_items),
                                                );
                                            }
                                        }
                                    }
                                }
                            }

                            // Process regular text content
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                if !output.is_empty() && !has_todo_update {
                                    output.push('\n');
                                }
                                output.push_str(text);
                            }
                        }

                        if !output.is_empty() {
                            if has_todo_update {
                                Some(output)
                            } else {
                                Some(format!("🤖 Assistant:\n{}", output))
                            }
                        } else {
                            None
                        }
                    }
                    Value::String(text) => Some(format!("🤖 Assistant:\n{}", text)),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    fn format_todo_update(&self, todos: &[TodoItem]) -> String {
        let mut output = String::new();
        output.push_str("📝 TODO Update:\n");
        output.push_str(&"─".repeat(60));
        output.push('\n');

        // Group todos by status
        let mut pending_todos = Vec::new();
        let mut in_progress_todos = Vec::new();
        let mut completed_todos = Vec::new();

        for todo in todos {
            match todo.status.as_str() {
                "pending" => pending_todos.push(todo),
                "in_progress" => in_progress_todos.push(todo),
                "completed" => completed_todos.push(todo),
                _ => pending_todos.push(todo), // Default to pending for unknown statuses
            }
        }

        // Display in-progress todos first
        if !in_progress_todos.is_empty() {
            output.push_str("🔄 In Progress:\n");
            for todo in &in_progress_todos {
                output.push_str(&format!(
                    "   {} [{}] {}\n",
                    self.get_priority_emoji(&todo.priority),
                    todo.id,
                    todo.content
                ));
            }
            output.push('\n');
        }

        // Display pending todos
        if !pending_todos.is_empty() {
            output.push_str("⏳ Pending:\n");
            for todo in &pending_todos {
                output.push_str(&format!(
                    "   {} [{}] {}\n",
                    self.get_priority_emoji(&todo.priority),
                    todo.id,
                    todo.content
                ));
            }
            output.push('\n');
        }

        // Display completed todos
        if !completed_todos.is_empty() {
            output.push_str("✅ Completed:\n");
            for todo in &completed_todos {
                output.push_str(&format!(
                    "   {} [{}] {}\n",
                    self.get_priority_emoji(&todo.priority),
                    todo.id,
                    todo.content
                ));
            }
            output.push('\n');
        }

        output.push_str(&"─".repeat(60));
        output.push('\n');

        // Add summary
        let total = todos.len();
        let completed = completed_todos.len();
        let in_progress = in_progress_todos.len();
        let pending = pending_todos.len();

        output.push_str(&format!(
            "Summary: {} total | {} completed | {} in progress | {} pending\n",
            total, completed, in_progress, pending
        ));
        output.push_str(&"─".repeat(60));

        output
    }

    fn get_priority_emoji(&self, priority: &str) -> &'static str {
        match priority {
            "high" => "🔴",
            "medium" => "🟡",
            "low" => "🟢",
            _ => "⚪",
        }
    }

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

            self.final_result = Some(TaskResult {
                success,
                message: message.clone(),
                cost_usd: msg.cost_usd,
                duration_ms: msg.duration_ms,
            });

            // Format a nice summary
            let mut output = String::new();
            output.push('\n');
            output.push_str(&"─".repeat(60));
            output.push('\n');

            let status_emoji = if success { "✅" } else { "❌" };
            output.push_str(&format!("{} Task Result: {}\n", status_emoji, subtype));

            if let Some(cost) = msg.cost_usd {
                output.push_str(&format!("💰 Cost: ${:.2}\n", cost));
            }

            if let Some(duration) = msg.duration_ms {
                let seconds = duration as f64 / 1000.0;
                output.push_str(&format!("⏱️ Duration: {:.1}s\n", seconds));
            }

            if let Some(turns) = msg.num_turns {
                output.push_str(&format!("🔄 Turns: {}\n", turns));
            }

            output.push_str(&"─".repeat(60));

            Some(output)
        } else {
            None
        }
    }

    #[allow(dead_code)] // Available for future use
    pub fn get_full_log(&self) -> String {
        self.full_log.join("\n")
    }

    pub async fn save_full_log(&self, path: &std::path::Path) -> Result<(), String> {
        let content = self.full_log.join("\n");
        if let Some(ref fs) = self.file_system {
            fs.write_file(path, &content)
                .await
                .map_err(|e| format!("Failed to save log file: {}", e))
        } else {
            Err("File system not initialized in LogProcessor".to_string())
        }
    }

    pub fn get_final_result(&self) -> Option<&TaskResult> {
        self.final_result.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_assistant_message() {
        let mut processor = LogProcessor::new();
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "Hello, world!"}]
            }
        }"#;

        let result = processor.process_line(json);
        assert_eq!(result, Some("🤖 Assistant:\nHello, world!".to_string()));
    }

    #[test]
    fn test_process_result_message() {
        let mut processor = LogProcessor::new();
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
        let mut processor = LogProcessor::new();
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
        assert!(formatted.contains("⏱️ Duration: 5.0s"));

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
        let mut processor = LogProcessor::new();
        let line = "This is not JSON";

        let result = processor.process_line(line);
        assert_eq!(result, Some("‼️ parsing error".to_string()));
    }

    #[test]
    fn test_process_other_message_types() {
        let mut processor = LogProcessor::new();

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
        let mut processor = LogProcessor::new();
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

        // Check that todos are grouped by status
        assert!(formatted.contains("🔄 In Progress:"));
        assert!(formatted.contains("⏳ Pending:"));
        assert!(formatted.contains("✅ Completed:"));

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
        let mut processor = LogProcessor::new();
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

        // Should only have completed section
        assert!(formatted.contains("✅ Completed:"));
        assert!(!formatted.contains("🔄 In Progress:"));
        assert!(!formatted.contains("⏳ Pending:"));

        // Check summary
        assert!(formatted.contains("Summary: 2 total | 2 completed | 0 in progress | 0 pending"));
    }

    #[test]
    fn test_todo_priority_emojis() {
        let processor = LogProcessor::new();
        assert_eq!(processor.get_priority_emoji("high"), "🔴");
        assert_eq!(processor.get_priority_emoji("medium"), "🟡");
        assert_eq!(processor.get_priority_emoji("low"), "🟢");
        assert_eq!(processor.get_priority_emoji("unknown"), "⚪");
    }
}
