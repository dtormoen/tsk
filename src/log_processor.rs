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
}

impl LogProcessor {
    pub fn new() -> Self {
        Self {
            full_log: Vec::new(),
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

    fn format_message(&self, msg: ClaudeMessage) -> Option<String> {
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

    fn format_result_message(&self, msg: ClaudeMessage) -> Option<String> {
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
            "cost_usd": 0.123
        }"#;

        let result = processor.process_line(json);
        assert!(result.is_some());
        assert!(result.unwrap().contains("\"type\": \"result\""));
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
