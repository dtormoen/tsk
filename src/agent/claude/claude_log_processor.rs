use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::agent::{LogProcessor, TaskResult};

/// Represents a message from Claude's JSON output
#[derive(Debug, Deserialize, Serialize)]
struct ClaudeMessage {
    #[serde(rename = "type")]
    message_type: String,
    message: Option<MessageContent>,
    subtype: Option<String>,
    cost_usd: Option<f64>,
    is_error: Option<bool>,
    duration_ms: Option<u64>,
    num_turns: Option<u64>,
    result: Option<String>,
    #[serde(rename = "toolUseResult")]
    tool_use_result: Option<Value>,
    summary: Option<String>,
    parent_tool_use_id: Option<String>,
}

/// Message content structure from Claude
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

/// Usage information from Claude
#[derive(Debug, Deserialize, Serialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    service_tier: Option<String>,
}

/// Todo item structure from Claude's TodoWrite tool
#[derive(Debug, Deserialize, Serialize, Clone)]
struct TodoItem {
    content: String,
    status: String,
    #[serde(rename = "activeForm")]
    active_form: Option<String>,
    #[serde(default)]
    priority: Option<String>,
}

/// Task invocation information for tracking sub-agent tasks
#[derive(Debug, Clone)]
struct TaskInvocation {
    subagent_type: String,
    description: String,
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
/// - Task tool tracking with sub-agent output display
///
/// The processor handles non-JSON output gracefully:
/// - Initially prints non-JSON lines as-is (for misconfiguration messages)
/// - Switches to JSON-only mode after the first valid JSON line
pub struct ClaudeLogProcessor {
    full_log: Vec<String>,
    final_result: Option<TaskResult>,
    json_mode_active: bool,
    /// Track whether the previous line was a parsing error to avoid duplicate error messages
    last_line_was_parse_error: bool,
    /// Optional task name for prefixing log lines
    task_name: Option<String>,
    /// Track task contexts by parent_tool_use_id for sub-agent message tagging
    task_contexts: HashMap<String, TaskInvocation>,
}

impl ClaudeLogProcessor {
    /// Creates a new ClaudeLogProcessor
    pub fn new(task_name: Option<String>) -> Self {
        Self {
            full_log: Vec::new(),
            final_result: None,
            json_mode_active: false,
            last_line_was_parse_error: false,
            task_name,
            task_contexts: HashMap::new(),
        }
    }

    /// Creates a prefix for log lines in the format: <emoji> [<Task.name>][<model>][<sub-agent-name>]:
    fn create_prefix(
        &self,
        emoji: &str,
        model: Option<&str>,
        sub_agent: Option<&str>,
        parent_tool_use_id: Option<&str>,
    ) -> String {
        let mut prefix = emoji.to_string();

        if let Some(task_name) = &self.task_name {
            prefix.push_str(&format!(" [{}]", task_name));
        }

        if let Some(model) = model {
            prefix.push_str(&format!("[{}]", model));
        }

        // Two-level fallback for sub_agent name:
        // 1. Use explicit sub_agent parameter if provided
        // 2. Lookup parent_tool_use_id in task_contexts
        let sub_agent_name = sub_agent.or_else(|| {
            parent_tool_use_id
                .and_then(|id| self.task_contexts.get(id))
                .map(|task| task.subagent_type.as_str())
        });

        if let Some(sub_agent_name) = sub_agent_name {
            prefix.push_str(&format!("[{}]", sub_agent_name));
        }

        prefix.push_str(": ");
        prefix
    }

    /// Formats a Claude message based on its type
    fn format_message(&mut self, msg: ClaudeMessage) -> Option<String> {
        let parent_tool_use_id = msg.parent_tool_use_id.as_deref();

        match msg.message_type.as_str() {
            "assistant" => self.format_assistant_message(msg),
            "user" => self.format_user_message(&msg),
            "result" => self.format_result_message(msg),
            "summary" => {
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
                    Some(format!(
                        "{}{summary_text}",
                        self.create_prefix("✅", None, None, parent_tool_use_id)
                    ))
                } else {
                    Some(format!(
                        "{}Task Complete",
                        self.create_prefix("✅", None, None, parent_tool_use_id)
                    ))
                }
            }
            other_type => {
                // For other message types, just show a brief indicator
                Some(format!(
                    "{}[{other_type}]",
                    self.create_prefix("📋", None, None, parent_tool_use_id)
                ))
            }
        }
    }

    /// Extracts the actual content from a tool_result item in the message content array
    fn extract_tool_result_content(&self, item: &Value) -> Option<String> {
        if let Some(content) = item.get("content") {
            match content {
                // If content is a string, use it directly
                Value::String(text) => Some(text.clone()),
                // If content is an array, iterate through all elements looking for text objects
                Value::Array(arr) => {
                    let mut text_parts = Vec::new();

                    for element in arr {
                        // Look for objects with "type": "text" and extract their "text" field
                        if let Some(element_type) = element.get("type")
                            && element_type.as_str() == Some("text")
                            && let Some(text) = element.get("text")
                            && let Some(text_str) = text.as_str()
                        {
                            text_parts.push(text_str);
                        }
                    }

                    if text_parts.is_empty() {
                        None
                    } else {
                        Some(text_parts.join(""))
                    }
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Formats a user message with tool result information
    fn format_user_message(&mut self, msg: &ClaudeMessage) -> Option<String> {
        let parent_tool_use_id = msg.parent_tool_use_id.as_deref();

        // Check if this is a tool result message
        if let Some(message) = &msg.message
            && let Some(content) = &message.content
        {
            // Check if content is an array with tool_result
            if let Value::Array(contents) = content {
                for item in contents {
                    if let Some("tool_result") = item.get("type").and_then(|t| t.as_str()) {
                        let tool_use_id = item.get("tool_use_id").and_then(|id| id.as_str());

                        // Check for empty tool results - filter them out UNLESS it's a Task tool result
                        let is_task_tool_result = tool_use_id
                            .map(|id| self.task_contexts.contains_key(id))
                            .unwrap_or(false);

                        if !is_task_tool_result {
                            let is_empty = if let Some(content) = item.get("content") {
                                match content {
                                    Value::String(text) => text.trim().is_empty(),
                                    Value::Array(arr) => arr.is_empty(),
                                    _ => false,
                                }
                            } else {
                                true // No content field
                            };

                            // For non-Task tools with empty content, filter out completely
                            if is_empty {
                                return None;
                            }
                        }

                        // Check if this is a Task tool result first (by tool_use_id in task_contexts)
                        if let Some(tool_use_id) = tool_use_id
                            && let Some(task_info) = self.task_contexts.get(tool_use_id)
                        {
                            // Extract the actual content from the tool_result item
                            let actual_content = self.extract_tool_result_content(item);

                            let result = self.format_task_tool_result(
                                task_info,
                                parent_tool_use_id,
                                actual_content.as_deref(),
                            );

                            // Remove the completed task from tracking
                            self.task_contexts.remove(tool_use_id);

                            return Some(result);
                        }

                        // For non-Task tools, try to determine which tool was used from toolUseResult
                        if let Some(tool_result) = msg.tool_use_result.as_ref() {
                            // Extract the actual content from the tool_result item
                            let actual_content = self.extract_tool_result_content(item);

                            let result = self.format_tool_result(
                                tool_result,
                                tool_use_id,
                                parent_tool_use_id,
                                actual_content.as_deref(),
                            );

                            return Some(result);
                        }

                        // If we have a tool_result but no toolUseResult in the message,
                        // it's likely from a sub-agent with no useful content, so filter it out
                        return None;
                    }
                }
            }
            // Check if it's a regular user message with string content
            else if let Value::String(text) = content {
                let first_line = text.lines().next().unwrap_or("").trim();
                if first_line.starts_with("# ") {
                    return Some(format!(
                        "{}User: {}",
                        self.create_prefix("👤", None, None, parent_tool_use_id),
                        first_line.trim_start_matches("# ")
                    ));
                } else if first_line.len() > 60 {
                    return Some(format!(
                        "{}User: {}...",
                        self.create_prefix("👤", None, None, parent_tool_use_id),
                        &first_line[..60]
                    ));
                } else if !first_line.is_empty() {
                    return Some(format!(
                        "{}User: {first_line}",
                        self.create_prefix("👤", None, None, parent_tool_use_id)
                    ));
                }
            }
        }

        // Default fallback
        Some(format!(
            "{}[user]",
            self.create_prefix("👤", None, None, parent_tool_use_id)
        ))
    }

    /// Formats tool result based on its content
    fn format_tool_result(
        &mut self,
        tool_result: &Value,
        tool_use_id: Option<&str>,
        parent_tool_use_id: Option<&str>,
        actual_content: Option<&str>,
    ) -> String {
        // Check if this is a Task tool result with sub-agent output
        if let Some(tool_use_id) = tool_use_id
            && let Some(task_info) = self.task_contexts.get(tool_use_id)
        {
            let prefix = self.create_prefix(
                "🤖",
                None,
                Some(&task_info.subagent_type),
                parent_tool_use_id,
            );

            // Use the actual content from the message instead of tool_result
            if let Some(content) = actual_content {
                // If content is not empty, display it properly
                if !content.trim().is_empty() {
                    // Convert escaped newlines to actual newlines for better display
                    let processed_content = content.replace("\\n", "\n");
                    return format!(
                        "{}Task Complete: {} ({})\n\n📋 Sub-agent Output:\n{}",
                        prefix, task_info.description, task_info.subagent_type, processed_content
                    );
                }
            }

            // Fallback for empty or missing content
            return format!(
                "{}Task Complete: {} ({})",
                prefix, task_info.description, task_info.subagent_type
            );
        }
        // Check for specific result types to identify the tool
        if tool_result.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(file_info) = tool_result.get("file")
                && let Some(path) = file_info.get("filePath").and_then(|p| p.as_str())
            {
                let filename = path.rsplit('/').next().unwrap_or(path);
                return format!(
                    "{}Read result: {filename}",
                    self.create_prefix("📖", None, None, parent_tool_use_id)
                );
            }
            return format!(
                "{}Read result",
                self.create_prefix("📖", None, None, parent_tool_use_id)
            );
        }

        if tool_result.get("filePath").is_some() {
            return format!(
                "{}Edit result",
                self.create_prefix("✏️", None, None, parent_tool_use_id)
            );
        }

        if tool_result.get("oldTodos").is_some() || tool_result.get("newTodos").is_some() {
            // For TodoWrite results, show the todo update
            if let Some(new_todos) = tool_result.get("newTodos")
                && let Ok(todos) = serde_json::from_value::<Vec<TodoItem>>(new_todos.clone())
            {
                return format!(
                    "{}TodoWrite result - {} todos",
                    self.create_prefix("📝", None, None, parent_tool_use_id),
                    todos.len()
                );
            }
            return format!(
                "{}TodoWrite result",
                self.create_prefix("📝", None, None, parent_tool_use_id)
            );
        }

        if let Some(filenames) = tool_result.get("filenames").and_then(|v| v.as_array()) {
            let count = filenames.len();
            return format!(
                "{}Search result: Found {} file{}",
                self.create_prefix("🔍", None, None, parent_tool_use_id),
                count,
                if count == 1 { "" } else { "s" }
            );
        }

        if let Some(stdout) = tool_result.get("stdout").and_then(|v| v.as_str()) {
            if stdout.contains("test result: ok") {
                return format!(
                    "{}Bash result: Tests passed ✅",
                    self.create_prefix("🖥️", None, None, parent_tool_use_id)
                );
            } else if stdout.contains("test result: FAILED") {
                return format!(
                    "{}Bash result: Tests failed ❌",
                    self.create_prefix("🖥️", None, None, parent_tool_use_id)
                );
            } else if stdout.trim().is_empty() {
                return format!(
                    "{}Bash result: Command completed",
                    self.create_prefix("🖥️", None, None, parent_tool_use_id)
                );
            } else {
                let first_line = stdout.lines().next().unwrap_or("").trim();
                if first_line.len() > 40 {
                    return format!(
                        "{}Bash result: {}...",
                        self.create_prefix("🖥️", None, None, parent_tool_use_id),
                        &first_line[..40]
                    );
                } else {
                    return format!(
                        "{}Bash result: {first_line}",
                        self.create_prefix("🖥️", None, None, parent_tool_use_id)
                    );
                }
            }
        }

        if let Some(error) = tool_result.get("error").and_then(|v| v.as_str()) {
            return format!(
                "{}Tool error: {error}",
                self.create_prefix("❌", None, None, parent_tool_use_id)
            );
        }

        if tool_result.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
            return format!(
                "{}Tool error occurred",
                self.create_prefix("❌", None, None, parent_tool_use_id)
            );
        }

        format!(
            "{}Tool result: Completed",
            self.create_prefix("🔧", None, None, parent_tool_use_id)
        )
    }

    /// Formats a Task tool result with sub-agent output
    fn format_task_tool_result(
        &self,
        task_info: &TaskInvocation,
        parent_tool_use_id: Option<&str>,
        actual_content: Option<&str>,
    ) -> String {
        let prefix = self.create_prefix(
            "🤖",
            None,
            Some(&task_info.subagent_type),
            parent_tool_use_id,
        );

        // Use the actual content from the message instead of tool_result
        if let Some(content) = actual_content {
            // If content is not empty, display it properly
            if !content.trim().is_empty() {
                // Convert escaped newlines to actual newlines for better display
                let processed_content = content.replace("\\n", "\n");
                return format!(
                    "{}Task Complete: {} ({})\n\n📋 Sub-agent Output:\n{}",
                    prefix, task_info.description, task_info.subagent_type, processed_content
                );
            }
        }

        // Fallback for empty or missing content
        format!(
            "{}Task Complete: {} ({})",
            prefix, task_info.description, task_info.subagent_type
        )
    }

    /// Formats an assistant message, extracting text content, tool uses, and todo updates
    fn format_assistant_message(&mut self, msg: ClaudeMessage) -> Option<String> {
        let parent_tool_use_id = msg.parent_tool_use_id.as_deref();

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
                                // Store Task tool invocations for later matching with results
                                if tool_name == "Task"
                                    && let Some(tool_use_id) =
                                        item.get("id").and_then(|id| id.as_str())
                                    && let Some(input) = item.get("input")
                                {
                                    let subagent_type = input
                                        .get("subagent_type")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();
                                    let description = input
                                        .get("description")
                                        .and_then(|d| d.as_str())
                                        .unwrap_or("No description")
                                        .to_string();

                                    let task_invocation = TaskInvocation {
                                        subagent_type,
                                        description,
                                    };

                                    // Add to task_contexts for tracking messages by parent_tool_use_id
                                    self.task_contexts
                                        .insert(tool_use_id.to_string(), task_invocation);
                                }

                                let tool_output = self.format_tool_use(
                                    tool_name,
                                    &item,
                                    message.model.as_deref(),
                                    parent_tool_use_id,
                                );
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
                                let prefix = self.create_prefix(
                                    "🤖",
                                    message.model.as_deref(),
                                    None,
                                    parent_tool_use_id,
                                );
                                output.push_str(&format!("{}{text}", prefix));
                            }
                        }

                        if !output.is_empty() {
                            Some(output.trim_end().to_string())
                        } else {
                            None
                        }
                    }
                    Value::String(text) => {
                        let prefix = self.create_prefix(
                            "🤖",
                            message.model.as_deref(),
                            None,
                            parent_tool_use_id,
                        );
                        Some(format!("{}{text}", prefix))
                    }
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
    fn format_tool_use(
        &self,
        tool_name: &str,
        item: &Value,
        model: Option<&str>,
        parent_tool_use_id: Option<&str>,
    ) -> String {
        match tool_name {
            "Task" => {
                if let Some(input) = item.get("input") {
                    let subagent_type = input
                        .get("subagent_type")
                        .and_then(|s| s.as_str())
                        .unwrap_or("unknown");
                    let description = input
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("No description");
                    let prompt = input.get("prompt").and_then(|p| p.as_str()).unwrap_or("");

                    let prefix = self.create_prefix("🤖", model, None, parent_tool_use_id);
                    if !prompt.is_empty() {
                        format!(
                            "{}Starting Task: {} ({})\n\nPrompt: {}",
                            prefix, description, subagent_type, prompt
                        )
                    } else {
                        format!(
                            "{}Starting Task: {} ({})",
                            prefix, description, subagent_type
                        )
                    }
                } else {
                    let prefix = self.create_prefix("🤖", model, None, parent_tool_use_id);
                    format!("{}Starting Task", prefix)
                }
            }
            "TodoWrite" => {
                if let Some(input) = item.get("input")
                    && let Some(todos) = input.get("todos")
                    && let Ok(todo_items) = serde_json::from_value::<Vec<TodoItem>>(todos.clone())
                {
                    self.format_todo_update(&todo_items, parent_tool_use_id)
                } else {
                    let prefix = self.create_prefix("📝", model, None, parent_tool_use_id);
                    format!("{}Using TodoWrite\n", prefix)
                }
            }
            "Read" => {
                if let Some(input) = item.get("input")
                    && let Some(file_path) = input.get("file_path").and_then(|f| f.as_str())
                {
                    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
                    let prefix = self.create_prefix("📖", model, None, parent_tool_use_id);
                    format!("{}Reading {file_name}\n", prefix)
                } else {
                    let prefix = self.create_prefix("📖", model, None, parent_tool_use_id);
                    format!("{}Reading file\n", prefix)
                }
            }
            "NotebookRead" => {
                let prefix = self.create_prefix("📓", model, None, parent_tool_use_id);
                format!("{}Reading notebook\n", prefix)
            }
            "Edit" | "MultiEdit" => {
                if let Some(input) = item.get("input")
                    && let Some(file_path) = input.get("file_path").and_then(|f| f.as_str())
                {
                    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
                    let prefix = self.create_prefix("🔧", model, None, parent_tool_use_id);
                    if tool_name == "MultiEdit" {
                        if let Some(edits) = input.get("edits").and_then(|e| e.as_array()) {
                            format!("{}Editing {file_name} ({} changes)\n", prefix, edits.len())
                        } else {
                            format!("{}Editing {file_name}\n", prefix)
                        }
                    } else {
                        format!("{}Editing {file_name}\n", prefix)
                    }
                } else {
                    let prefix = self.create_prefix("🔧", model, None, parent_tool_use_id);
                    format!("{}Using {tool_name}\n", prefix)
                }
            }
            "Bash" => {
                if let Some(input) = item.get("input")
                    && let Some(cmd) = input.get("command").and_then(|c| c.as_str())
                {
                    // Show the full command, not just a preview
                    let prefix = self.create_prefix("🖥️", model, None, parent_tool_use_id);
                    format!("{}Running: {cmd}\n", prefix)
                } else {
                    let prefix = self.create_prefix("🖥️", model, None, parent_tool_use_id);
                    format!("{}Running command\n", prefix)
                }
            }
            "Write" => {
                if let Some(input) = item.get("input")
                    && let Some(file_path) = input.get("file_path").and_then(|f| f.as_str())
                {
                    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
                    let prefix = self.create_prefix("📝", model, None, parent_tool_use_id);
                    format!("{}Writing {file_name}\n", prefix)
                } else {
                    let prefix = self.create_prefix("📝", model, None, parent_tool_use_id);
                    format!("{}Writing file\n", prefix)
                }
            }
            "Grep" => {
                if let Some(input) = item.get("input")
                    && let Some(pattern) = input.get("pattern").and_then(|p| p.as_str())
                {
                    let prefix = self.create_prefix("🔍", model, None, parent_tool_use_id);
                    format!("{}Searching for: {pattern}\n", prefix)
                } else {
                    let prefix = self.create_prefix("🔍", model, None, parent_tool_use_id);
                    format!("{}Searching\n", prefix)
                }
            }
            "WebSearch" => {
                if let Some(input) = item.get("input")
                    && let Some(query) = input.get("query").and_then(|q| q.as_str())
                {
                    let prefix = self.create_prefix("🌐", model, None, parent_tool_use_id);
                    format!("{}Web search: {query}\n", prefix)
                } else {
                    let prefix = self.create_prefix("🌐", model, None, parent_tool_use_id);
                    format!("{}Web search\n", prefix)
                }
            }
            _ => {
                // For unknown tools, show description if available
                let description = if let Some(input) = item.get("input") {
                    if let Some(desc) = input.get("description").and_then(|d| d.as_str()) {
                        format!(": {desc}")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                let prefix = self.create_prefix("🔧", model, None, parent_tool_use_id);
                format!("{}Using {tool_name}{description}\n", prefix)
            }
        }
    }

    /// Formats a todo list update with status indicators and summary
    fn format_todo_update(&self, todos: &[TodoItem], parent_tool_use_id: Option<&str>) -> String {
        let mut output = String::new();
        let prefix = self.create_prefix("📝", None, None, parent_tool_use_id);
        output.push_str(&format!("{}TODO Update:\n", prefix));
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
impl LogProcessor for ClaudeLogProcessor {
    fn process_line(&mut self, line: &str) -> Option<String> {
        // Store the raw line for the full log
        self.full_log.push(line.to_string());

        // Skip empty lines - don't reset error tracking
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
                // Reset the error flag on successful parse
                self.last_line_was_parse_error = false;
                self.format_message(msg)
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
                        Some(format!(
                            "{}parsing error",
                            self.create_prefix("‼️", None, None, None)
                        ))
                    }
                } else {
                    // Before JSON mode is active, pass through non-JSON lines as-is
                    // Don't set last_line_was_parse_error since we're not in JSON mode yet
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
    use serde_json::json;

    // Simplified helper functions using serde_json::json! macro

    fn task_msg(id: &str, agent: &str, desc: &str, prompt: Option<&str>) -> String {
        let input = if let Some(p) = prompt {
            json!({"subagent_type": agent, "description": desc, "prompt": p})
        } else {
            json!({"subagent_type": agent, "description": desc})
        };
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": id, "name": "Task", "input": input}]}}).to_string()
    }

    fn task_result(id: &str, content: &str, parent_id: Option<&str>) -> String {
        let mut result = json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": id, "content": content}]}, "toolUseResult": {}});
        if let Some(pid) = parent_id {
            result["parent_tool_use_id"] = json!(pid);
        }
        result.to_string()
    }

    fn assistant_text(text: &str, model: Option<&str>, parent_id: Option<&str>) -> String {
        let mut msg =
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}});
        if let Some(m) = model {
            msg["message"]["model"] = json!(m);
        }
        if let Some(pid) = parent_id {
            msg["parent_tool_use_id"] = json!(pid);
        }
        msg.to_string()
    }

    fn tool_use_msg(
        tool: &str,
        input: &str,
        model: Option<&str>,
        parent_id: Option<&str>,
    ) -> String {
        let mut msg = json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": tool, "input": serde_json::from_str::<serde_json::Value>(input).unwrap()}]}});
        if let Some(m) = model {
            msg["message"]["model"] = json!(m);
        }
        if let Some(pid) = parent_id {
            msg["parent_tool_use_id"] = json!(pid);
        }
        msg.to_string()
    }

    fn todo_msg(todos: &str) -> String {
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "TodoWrite", "input": {"todos": serde_json::from_str::<serde_json::Value>(todos).unwrap()}}]}}).to_string()
    }

    // Simplified assertion helpers
    fn has_tag(output: &str, tag: &str) {
        assert!(output.contains(&format!("[{}]", tag)));
    }
    fn no_tag(output: &str, tag: &str) {
        assert!(!output.contains(&format!("[{}]", tag)));
    }

    #[test]
    fn test_todo_processing() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Test basic format with status icons and summary
        let todos = r#"[
            {"content": "Analyze current usage", "status": "in_progress", "activeForm": "Analyzing current usage"},
            {"content": "Move file to new location", "status": "pending", "activeForm": "Moving file to new location"},
            {"content": "Update imports", "status": "completed", "activeForm": "Updating imports"}
        ]"#;
        let msg = todo_msg(todos);
        let output = processor.process_line(&msg).unwrap();
        assert!(output.contains("📝 [test-task]: TODO Update:"));
        assert!(output.contains("🔄  [1] Analyzing current usage"));
        assert!(output.contains("⏳  [2] Move file to new location"));
        assert!(output.contains("✅  [3] Update imports"));
        assert!(output.contains("Summary: 3 total | 1 completed | 1 in progress | 1 pending"));

        // Test priority indicators
        let todos_with_priority = r#"[
            {"content": "High priority task", "status": "pending", "activeForm": "Working on high priority", "priority": "high"},
            {"content": "Low priority task", "status": "completed", "activeForm": "Completed low priority", "priority": "low"}
        ]"#;
        let msg = todo_msg(todos_with_priority);
        let output = processor.process_line(&msg).unwrap();
        assert!(output.contains("⏳ 🔴 [1]")); // pending with high priority
        assert!(output.contains("✅ 🟢 [2]")); // completed with low priority
    }

    #[test]
    fn test_user_message_with_todo_result() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));
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
        assert_eq!(
            result.unwrap(),
            "📝 [test-task]: TodoWrite result - 2 todos"
        );
    }

    #[test]
    fn test_assistant_message_formats() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Text message without model
        let json = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello, world!"}]}}"#;
        let output = processor.process_line(json).unwrap();
        assert_eq!(output, "🤖 [test-task]: Hello, world!");

        // Text message with model
        let json = r#"{"type": "assistant", "message": {"model": "claude-opus", "content": [{"type": "text", "text": "Hello with model!"}]}}"#;
        let output = processor.process_line(json).unwrap();
        assert_eq!(output, "🤖 [test-task][claude-opus]: Hello with model!");

        // Direct string content with model
        let json = r#"{"type": "assistant", "message": {"model": "claude-3-sonnet", "content": "Direct string content"}}"#;
        let output = processor.process_line(json).unwrap();
        assert_eq!(
            output,
            "🤖 [test-task][claude-3-sonnet]: Direct string content"
        );

        // Bash tool use
        let json = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Bash", "input": {"command": "cargo test"}}]}}"#;
        let output = processor.process_line(json).unwrap();
        assert_eq!(output, "🖥️ [test-task]: Running: cargo test");
    }

    #[test]
    fn test_result_and_summary_messages() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Test result message with cost and duration
        let result_json = r#"{
            "type": "result",
            "subtype": "success",
            "cost_usd": 0.15,
            "duration_ms": 45000,
            "result": "Task completed successfully"
        }"#;
        let output = processor.process_line(result_json).unwrap();
        assert!(output.contains("✅ Task Result: success"));
        assert!(output.contains("💰 Cost: $0.15"));
        assert!(output.contains("⏱️ Duration: 45 seconds"));

        let final_result = processor.get_final_result().unwrap();
        assert!(final_result.success);
        assert_eq!(final_result.cost_usd, Some(0.15));

        // Test summary message
        let summary_json = r#"{
            "type": "summary",
            "summary": "Refactoring completed: Renamed XdgDirectories to TskConfig"
        }"#;
        let output = processor.process_line(summary_json).unwrap();
        assert_eq!(
            output,
            "✅ [test-task]: Refactoring completed: Renamed XdgDirectories to TskConfig"
        );
    }

    #[test]
    fn test_json_mode_behavior() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Initially not in JSON mode
        assert!(!processor.json_mode_active);

        // Before JSON mode is active, non-JSON lines should be passed through as-is
        let result = processor.process_line("Error: Claude Code is misconfigured");
        assert_eq!(
            result,
            Some("Error: Claude Code is misconfigured".to_string())
        );
        assert!(!processor.json_mode_active);

        let result = processor.process_line("Configuration warning: API key missing");
        assert_eq!(
            result,
            Some("Configuration warning: API key missing".to_string())
        );
        assert!(!processor.json_mode_active);

        // First valid JSON line activates JSON mode
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello"}]}}"#;
        let result = processor.process_line(json);
        assert_eq!(result, Some("🤖 [test-task]: Hello".to_string()));
        assert!(processor.json_mode_active);

        // After JSON mode is active, non-JSON lines should show parsing error
        let result = processor.process_line("This is not JSON");
        assert_eq!(result, Some("‼️ [test-task]: parsing error".to_string()));

        // Consecutive parsing errors should be suppressed
        let result = processor.process_line("random text");
        assert_eq!(result, None);
    }

    #[test]
    fn test_empty_tool_results_filtered() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Test empty tool result (should return None)
        let empty_tool_result_json = r#"{
            "type": "user",
            "message": {
                "role": "user",
                "content": [{"tool_use_id": "toolu_456", "type": "tool_result", "content": ""}]
            }
        }"#;

        let result = processor.process_line(empty_tool_result_json);
        // Should return None for empty tool results, not show "🔧 Tool result"
        assert!(result.is_none());
    }

    #[test]
    fn test_sub_agent_tagging_scenarios() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Task invocation should NOT have sub-agent tag
        let msg = task_msg(
            "toolu_123",
            "software-architect",
            "Test task",
            Some("Test prompt"),
        );
        let output = processor.process_line(&msg).unwrap();
        no_tag(&output, "software-architect");

        // Messages with parent_tool_use_id should have sub-agent tag
        let msg = tool_use_msg("Bash", r#"{"command": "ls -la"}"#, None, Some("toolu_123"));
        let output = processor.process_line(&msg).unwrap();
        has_tag(&output, "software-architect");

        let msg = assistant_text("Working on task", None, Some("toolu_123"));
        let output = processor.process_line(&msg).unwrap();
        has_tag(&output, "software-architect");

        // Task result should have sub-agent tag
        let result = task_result("toolu_123", "Task completed", None);
        let output = processor.process_line(&result).unwrap();
        has_tag(&output, "software-architect");

        // Summary without parent should NOT have tag
        let summary = r#"{"type":"summary","summary":"Task completed successfully"}"#;
        if let Some(output) = processor.process_line(summary) {
            no_tag(&output, "software-architect");
        }
    }

    #[test]
    fn test_task_tool_scenarios() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Test with prompt
        let msg1 = task_msg(
            "toolu_123",
            "software-architect",
            "Analyze code",
            Some("Check patterns"),
        );
        let output = processor.process_line(&msg1).unwrap();
        assert!(
            output.contains("🤖 [test-task]: Starting Task: Analyze code (software-architect)")
        );
        assert!(output.contains("Prompt: Check patterns"));

        let result = task_result("toolu_123", "Analysis complete", None);
        let output = processor.process_line(&result).unwrap();
        assert!(output.contains("Task Complete: Analyze code (software-architect)"));

        // Test array content with sub-agent output
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));
        let msg2 = task_msg("toolu_456", "data-analyst", "Analyze patterns", None);
        processor.process_line(&msg2);

        let result = task_result(
            "toolu_456",
            r#"[{"type": "text", "text": "Found 5 patterns. Pattern A: 45%."}]"#,
            None,
        );
        let output = processor.process_line(&result).unwrap();
        assert!(output.contains("Task Complete: Analyze patterns (data-analyst)"));
        assert!(output.contains("📋 Sub-agent Output:"));
        assert!(output.contains("Found 5 patterns"));

        // Test empty content (no sub-agent output section)
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));
        let msg3 = task_msg("toolu_789", "test-agent", "Test edge cases", None);
        processor.process_line(&msg3);

        let result = task_result("toolu_789", "", None);
        let output = processor.process_line(&result).unwrap();
        assert!(output.contains("Task Complete: Test edge cases (test-agent)"));
        assert!(!output.contains("📋 Sub-agent Output:"));
    }

    #[test]
    fn test_parse_error_deduplication() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // First, activate JSON mode with a valid JSON line
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello"}]}}"#;
        let result = processor.process_line(json);
        assert_eq!(result, Some("🤖 [test-task]: Hello".to_string()));
        assert!(processor.json_mode_active);

        // First parsing error should be shown
        let result = processor.process_line("This is not JSON");
        assert_eq!(result, Some("‼️ [test-task]: parsing error".to_string()));

        // Second parsing error should be suppressed (returns None)
        let result = processor.process_line("Another non-JSON line");
        assert_eq!(result, None);

        // Third parsing error should also be suppressed
        let result = processor.process_line("Yet another non-JSON line");
        assert_eq!(result, None);

        // Fourth parsing error should also be suppressed
        let result = processor.process_line("Still more non-JSON");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_error_resume_after_valid_json() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Activate JSON mode
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "First"}]}}"#;
        processor.process_line(json);

        // First parsing error shown
        let result = processor.process_line("Bad line 1");
        assert_eq!(result, Some("‼️ [test-task]: parsing error".to_string()));

        // Consecutive errors suppressed
        let result = processor.process_line("Bad line 2");
        assert_eq!(result, None);
        let result = processor.process_line("Bad line 3");
        assert_eq!(result, None);

        // Valid JSON resets the error flag
        let json = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Second"}]}}"#;
        let result = processor.process_line(json);
        assert_eq!(result, Some("🤖 [test-task]: Second".to_string()));

        // Next parsing error should be shown again
        let result = processor.process_line("Bad line 4");
        assert_eq!(result, Some("‼️ [test-task]: parsing error".to_string()));

        // And consecutive errors suppressed again
        let result = processor.process_line("Bad line 5");
        assert_eq!(result, None);
    }

    #[test]
    fn test_empty_lines_dont_reset_error_tracking() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Activate JSON mode
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Start"}]}}"#;
        processor.process_line(json);

        // First parsing error shown
        let result = processor.process_line("Bad line 1");
        assert_eq!(result, Some("‼️ [test-task]: parsing error".to_string()));

        // Empty line should not reset error tracking
        let result = processor.process_line("");
        assert_eq!(result, None);

        // Next parsing error should still be suppressed
        let result = processor.process_line("Bad line 2");
        assert_eq!(result, None);

        // Another empty line
        let result = processor.process_line("   ");
        assert_eq!(result, None);

        // Parsing error still suppressed
        let result = processor.process_line("Bad line 3");
        assert_eq!(result, None);

        // Valid JSON resets the state
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Good"}]}}"#;
        processor.process_line(json);

        // Empty line after valid JSON
        let result = processor.process_line("");
        assert_eq!(result, None);

        // Next parsing error should be shown (error flag was reset by valid JSON)
        let result = processor.process_line("Bad line 4");
        assert_eq!(result, Some("‼️ [test-task]: parsing error".to_string()));
    }

    #[test]
    fn test_pre_json_mode_behavior_unchanged() {
        let mut processor = ClaudeLogProcessor::new(Some("test-task".to_string()));

        // Before JSON mode, non-JSON lines should be passed through
        let result = processor.process_line("Configuration error message");
        assert_eq!(result, Some("Configuration error message".to_string()));
        assert!(!processor.json_mode_active);

        // Multiple non-JSON lines should all be passed through
        let result = processor.process_line("Warning: API key missing");
        assert_eq!(result, Some("Warning: API key missing".to_string()));
        assert!(!processor.json_mode_active);

        let result = processor.process_line("Another warning");
        assert_eq!(result, Some("Another warning".to_string()));
        assert!(!processor.json_mode_active);

        // Now activate JSON mode
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Start"}]}}"#;
        let result = processor.process_line(json);
        assert_eq!(result, Some("🤖 [test-task]: Start".to_string()));
        assert!(processor.json_mode_active);

        // Now parsing errors should be shown and deduplicated
        let result = processor.process_line("Bad line 1");
        assert_eq!(result, Some("‼️ [test-task]: parsing error".to_string()));

        let result = processor.process_line("Bad line 2");
        assert_eq!(result, None);
    }
}
