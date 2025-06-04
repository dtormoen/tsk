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
                        for item in contents {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                if !output.is_empty() {
                                    output.push('\n');
                                }
                                output.push_str(text);
                            }
                        }
                        if !output.is_empty() {
                            Some(format!("🤖 Assistant:\n{}", output))
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
}
