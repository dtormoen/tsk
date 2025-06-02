use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;

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
    pub fn new() -> Self {
        Self {
            full_log: Vec::new(),
            final_result: None,
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
                Some(line.to_string())
            }
        }
    }

    fn format_message(&mut self, msg: ClaudeMessage) -> Option<String> {
        match msg.message_type.as_str() {
            "assistant" => self.format_assistant_message(msg),
            "result" => self.format_result_message(msg),
            _ => Some(format!("Message type: {}", msg.message_type)),
        }
    }

    fn format_assistant_message(&self, msg: ClaudeMessage) -> Option<String> {
        if let Some(message) = msg.message {
            if let Some(content) = message.content {
                match content {
                    Value::Array(contents) => {
                        let mut output = String::new();
                        for item in contents {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                output.push_str(text);
                                output.push('\n');
                            }
                        }
                        if !output.is_empty() {
                            Some(output.trim_end().to_string())
                        } else {
                            None
                        }
                    }
                    Value::String(text) => Some(text),
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
                    msg.result
                        .clone()
                        .unwrap_or_else(|| "Task failed".to_string())
                }
            });

            self.final_result = Some(TaskResult {
                success,
                message,
                cost_usd: msg.cost_usd,
                duration_ms: msg.duration_ms,
            });
        }

        // Format the result message as pretty-printed JSON
        match serde_json::to_string_pretty(&msg) {
            Ok(json) => Some(json),
            Err(_) => Some(format!("Failed to format result message: {:?}", msg)),
        }
    }

    #[allow(dead_code)] // Available for future use
    pub fn get_full_log(&self) -> String {
        self.full_log.join("\n")
    }

    pub fn save_full_log(&self, path: &std::path::Path) -> std::io::Result<()> {
        let mut file = std::fs::File::create(path)?;
        for line in &self.full_log {
            writeln!(file, "{}", line)?;
        }
        Ok(())
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
        assert_eq!(result, Some("Hello, world!".to_string()));
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
        assert!(result.unwrap().contains("\"type\": \"result\""));

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

        // Check that the failure was parsed correctly
        let final_result = processor.get_final_result();
        assert!(final_result.is_some());
        let task_result = final_result.unwrap();
        assert_eq!(task_result.success, false);
        assert_eq!(task_result.message, "Task failed due to compilation errors");
        assert_eq!(task_result.duration_ms, Some(5000));
    }

    #[test]
    fn test_process_other_message() {
        let mut processor = LogProcessor::new();
        let json = r#"{"type": "user"}"#;

        let result = processor.process_line(json);
        assert_eq!(result, Some("Message type: user".to_string()));
    }

    #[test]
    fn test_process_non_json() {
        let mut processor = LogProcessor::new();
        let line = "This is not JSON";

        let result = processor.process_line(line);
        assert_eq!(result, Some(line.to_string()));
    }
}
