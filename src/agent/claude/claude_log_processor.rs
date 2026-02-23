use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[cfg(test)]
use crate::agent::log_line::Level;
use crate::agent::log_line::{LogLine, TodoItem, TodoStatus};
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

/// Todo item structure from Claude's TodoWrite tool (internal deserialization)
#[derive(Debug, Deserialize, Serialize, Clone)]
struct ClaudeTodoItem {
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
/// This processor produces structured `LogLine` values including:
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
    final_result: Option<TaskResult>,
    json_mode_active: bool,
    /// Track whether the previous line was a parsing error to avoid duplicate error messages
    last_line_was_parse_error: bool,
    /// Track task contexts by parent_tool_use_id for sub-agent message tagging
    task_contexts: HashMap<String, TaskInvocation>,
}

impl ClaudeLogProcessor {
    /// Creates a new ClaudeLogProcessor
    pub fn new() -> Self {
        Self {
            final_result: None,
            json_mode_active: false,
            last_line_was_parse_error: false,
            task_contexts: HashMap::new(),
        }
    }

    /// Build tags from model and parent_tool_use_id context.
    fn build_tags(
        &self,
        model: Option<&str>,
        sub_agent: Option<&str>,
        parent_tool_use_id: Option<&str>,
    ) -> Vec<String> {
        let mut tags = Vec::new();

        if let Some(model) = model {
            tags.push(model.to_string());
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
            tags.push(sub_agent_name.to_string());
        }

        tags
    }

    /// Formats a Claude message based on its type
    fn format_message(&mut self, msg: ClaudeMessage) -> Option<LogLine> {
        let parent_tool_use_id = msg.parent_tool_use_id.as_deref();

        match msg.message_type.as_str() {
            "assistant" => self.format_assistant_message(msg),
            "user" => self.format_user_message(&msg),
            "result" => self.format_result_message(msg),
            "summary" => {
                let tags = self.build_tags(None, None, parent_tool_use_id);
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
                    Some(LogLine::success(tags, None, summary_text))
                } else {
                    Some(LogLine::success(tags, None, "Task Complete".into()))
                }
            }
            other_type => {
                let tags = self.build_tags(None, None, parent_tool_use_id);
                Some(LogLine::message(tags, None, format!("[{other_type}]")))
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
    fn format_user_message(&mut self, msg: &ClaudeMessage) -> Option<LogLine> {
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
                let tags = self.build_tags(None, None, parent_tool_use_id);
                if first_line.starts_with("# ") {
                    return Some(LogLine::message(
                        tags,
                        None,
                        format!("User: {}", first_line.trim_start_matches("# ")),
                    ));
                } else if first_line.len() > 60 {
                    return Some(LogLine::message(
                        tags,
                        None,
                        format!("User: {}...", &first_line[..60]),
                    ));
                } else if !first_line.is_empty() {
                    return Some(LogLine::message(tags, None, format!("User: {first_line}")));
                }
            }
        }

        // Default fallback
        let tags = self.build_tags(None, None, parent_tool_use_id);
        Some(LogLine::message(tags, None, "[user]".into()))
    }

    /// Formats tool result based on its content
    fn format_tool_result(
        &mut self,
        tool_result: &Value,
        tool_use_id: Option<&str>,
        parent_tool_use_id: Option<&str>,
        actual_content: Option<&str>,
    ) -> LogLine {
        // Check if this is a Task tool result with sub-agent output
        if let Some(tool_use_id) = tool_use_id
            && let Some(task_info) = self.task_contexts.get(tool_use_id)
        {
            let tags = self.build_tags(None, Some(&task_info.subagent_type), parent_tool_use_id);

            // Use the actual content from the message instead of tool_result
            if let Some(content) = actual_content
                && !content.trim().is_empty()
            {
                let processed_content = content.replace("\\n", "\n");
                return LogLine::message(
                    tags,
                    Some("Task".into()),
                    format!(
                        "Complete: {} ({})\n\nSub-agent Output:\n{}",
                        task_info.description, task_info.subagent_type, processed_content
                    ),
                );
            }

            return LogLine::message(
                tags,
                Some("Task".into()),
                format!(
                    "Complete: {} ({})",
                    task_info.description, task_info.subagent_type
                ),
            );
        }

        let tags = self.build_tags(None, None, parent_tool_use_id);

        // Check for specific result types to identify the tool
        if tool_result.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(file_info) = tool_result.get("file")
                && let Some(path) = file_info.get("filePath").and_then(|p| p.as_str())
            {
                let filename = path.rsplit('/').next().unwrap_or(path);
                return LogLine::message(tags, Some("Read".into()), format!("result: {filename}"));
            }
            return LogLine::message(tags, Some("Read".into()), "result".into());
        }

        if tool_result.get("filePath").is_some() {
            return LogLine::message(tags, Some("Edit".into()), "result".into());
        }

        if tool_result.get("oldTodos").is_some() || tool_result.get("newTodos").is_some() {
            if let Some(new_todos) = tool_result.get("newTodos")
                && let Ok(todos) = serde_json::from_value::<Vec<ClaudeTodoItem>>(new_todos.clone())
            {
                return LogLine::message(
                    tags,
                    Some("TodoWrite".into()),
                    format!("result - {} todos", todos.len()),
                );
            }
            return LogLine::message(tags, Some("TodoWrite".into()), "result".into());
        }

        if let Some(filenames) = tool_result.get("filenames").and_then(|v| v.as_array()) {
            let count = filenames.len();
            return LogLine::message(
                tags,
                Some("Glob".into()),
                format!(
                    "result: Found {} file{}",
                    count,
                    if count == 1 { "" } else { "s" }
                ),
            );
        }

        if let Some(stdout) = tool_result.get("stdout").and_then(|v| v.as_str()) {
            if stdout.contains("test result: ok") {
                return LogLine::success(tags, Some("Bash".into()), "result: Tests passed".into());
            } else if stdout.contains("test result: FAILED") {
                return LogLine::error(tags, Some("Bash".into()), "result: Tests failed".into());
            } else if stdout.trim().is_empty() {
                return LogLine::message(
                    tags,
                    Some("Bash".into()),
                    "result: Command completed".into(),
                );
            } else {
                let first_line = stdout.lines().next().unwrap_or("").trim();
                if first_line.len() > 40 {
                    return LogLine::message(
                        tags,
                        Some("Bash".into()),
                        format!("result: {}...", &first_line[..40]),
                    );
                } else {
                    return LogLine::message(
                        tags,
                        Some("Bash".into()),
                        format!("result: {first_line}"),
                    );
                }
            }
        }

        if let Some(error) = tool_result.get("error").and_then(|v| v.as_str()) {
            return LogLine::error(tags, None, format!("Tool error: {error}"));
        }

        if tool_result.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
            return LogLine::error(tags, None, "Tool error occurred".into());
        }

        LogLine::message(tags, None, "Tool result: Completed".into())
    }

    /// Formats a Task tool result with sub-agent output
    fn format_task_tool_result(
        &self,
        task_info: &TaskInvocation,
        parent_tool_use_id: Option<&str>,
        actual_content: Option<&str>,
    ) -> LogLine {
        let tags = self.build_tags(None, Some(&task_info.subagent_type), parent_tool_use_id);

        // Use the actual content from the message
        if let Some(content) = actual_content
            && !content.trim().is_empty()
        {
            let processed_content = content.replace("\\n", "\n");
            return LogLine::message(
                tags,
                Some("Task".into()),
                format!(
                    "Complete: {} ({})\n\nSub-agent Output:\n{}",
                    task_info.description, task_info.subagent_type, processed_content
                ),
            );
        }

        // Fallback for empty or missing content
        LogLine::message(
            tags,
            Some("Task".into()),
            format!(
                "Complete: {} ({})",
                task_info.description, task_info.subagent_type
            ),
        )
    }

    /// Formats an assistant message, extracting text content, tool uses, and todo updates
    fn format_assistant_message(&mut self, msg: ClaudeMessage) -> Option<LogLine> {
        let parent_tool_use_id = msg.parent_tool_use_id.as_deref();

        if let Some(message) = msg.message {
            if let Some(content) = message.content {
                match content {
                    Value::Array(contents) => {
                        let mut lines: Vec<LogLine> = Vec::new();

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

                                    self.task_contexts
                                        .insert(tool_use_id.to_string(), task_invocation);
                                }

                                if let Some(tool_line) = self.format_tool_use(
                                    tool_name,
                                    &item,
                                    message.model.as_deref(),
                                    parent_tool_use_id,
                                ) {
                                    lines.push(tool_line);
                                }
                            }

                            // Process regular text content
                            if let Some(text) = item.get("text").and_then(|t| t.as_str())
                                && !text.trim().is_empty()
                            {
                                let tags = self.build_tags(
                                    message.model.as_deref(),
                                    None,
                                    parent_tool_use_id,
                                );
                                lines.push(LogLine::message(tags, None, text.to_string()));
                            }
                        }

                        if lines.len() == 1 {
                            Some(lines.remove(0))
                        } else if lines.len() > 1 {
                            // Merge multiple LogLines into a single Message with newlines
                            let merged_text = lines
                                .iter()
                                .map(|l: &LogLine| l.to_string())
                                .collect::<Vec<_>>()
                                .join("\n");
                            // Use the tags from the first line
                            let tags = match &lines[0] {
                                LogLine::Message { tags, .. } => tags.clone(),
                                LogLine::Todo { tags, .. } => tags.clone(),
                                _ => vec![],
                            };
                            Some(LogLine::message(tags, None, merged_text))
                        } else {
                            None
                        }
                    }
                    Value::String(text) => {
                        let tags =
                            self.build_tags(message.model.as_deref(), None, parent_tool_use_id);
                        Some(LogLine::message(tags, None, text))
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
    ) -> Option<LogLine> {
        let tags = self.build_tags(model, None, parent_tool_use_id);

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

                    if !prompt.is_empty() {
                        Some(LogLine::message(
                            tags,
                            Some("Task".into()),
                            format!(
                                "Starting: {} ({})\n\nPrompt: {}",
                                description, subagent_type, prompt
                            ),
                        ))
                    } else {
                        Some(LogLine::message(
                            tags,
                            Some("Task".into()),
                            format!("Starting: {} ({})", description, subagent_type),
                        ))
                    }
                } else {
                    Some(LogLine::message(
                        tags,
                        Some("Task".into()),
                        "Starting".into(),
                    ))
                }
            }
            "TodoWrite" => {
                if let Some(input) = item.get("input")
                    && let Some(todos) = input.get("todos")
                    && let Ok(todo_items) =
                        serde_json::from_value::<Vec<ClaudeTodoItem>>(todos.clone())
                {
                    Some(self.format_todo_update(&todo_items, parent_tool_use_id))
                } else {
                    Some(LogLine::message(
                        tags,
                        Some("TodoWrite".into()),
                        "update".into(),
                    ))
                }
            }
            "Read" => {
                if let Some(input) = item.get("input")
                    && let Some(file_path) = input.get("file_path").and_then(|f| f.as_str())
                {
                    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
                    Some(LogLine::message(
                        tags,
                        Some("Read".into()),
                        format!("Reading {file_name}"),
                    ))
                } else {
                    Some(LogLine::message(
                        tags,
                        Some("Read".into()),
                        "Reading file".into(),
                    ))
                }
            }
            "NotebookRead" => Some(LogLine::message(
                tags,
                Some("NotebookRead".into()),
                "Reading notebook".into(),
            )),
            "Edit" | "MultiEdit" => {
                if let Some(input) = item.get("input")
                    && let Some(file_path) = input.get("file_path").and_then(|f| f.as_str())
                {
                    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
                    if tool_name == "MultiEdit" {
                        if let Some(edits) = input.get("edits").and_then(|e| e.as_array()) {
                            Some(LogLine::message(
                                tags,
                                Some(tool_name.into()),
                                format!("Editing {file_name} ({} changes)", edits.len()),
                            ))
                        } else {
                            Some(LogLine::message(
                                tags,
                                Some(tool_name.into()),
                                format!("Editing {file_name}"),
                            ))
                        }
                    } else {
                        Some(LogLine::message(
                            tags,
                            Some("Edit".into()),
                            format!("Editing {file_name}"),
                        ))
                    }
                } else {
                    Some(LogLine::message(
                        tags,
                        Some(tool_name.into()),
                        format!("Using {tool_name}"),
                    ))
                }
            }
            "Bash" => {
                if let Some(input) = item.get("input")
                    && let Some(cmd) = input.get("command").and_then(|c| c.as_str())
                {
                    Some(LogLine::message(
                        tags,
                        Some("Bash".into()),
                        format!("Running: {cmd}"),
                    ))
                } else {
                    Some(LogLine::message(
                        tags,
                        Some("Bash".into()),
                        "Running command".into(),
                    ))
                }
            }
            "Write" => {
                if let Some(input) = item.get("input")
                    && let Some(file_path) = input.get("file_path").and_then(|f| f.as_str())
                {
                    let file_name = file_path.rsplit('/').next().unwrap_or(file_path);
                    Some(LogLine::message(
                        tags,
                        Some("Write".into()),
                        format!("Writing {file_name}"),
                    ))
                } else {
                    Some(LogLine::message(
                        tags,
                        Some("Write".into()),
                        "Writing file".into(),
                    ))
                }
            }
            "Grep" => {
                if let Some(input) = item.get("input")
                    && let Some(pattern) = input.get("pattern").and_then(|p| p.as_str())
                {
                    Some(LogLine::message(
                        tags,
                        Some("Grep".into()),
                        format!("Searching for: {pattern}"),
                    ))
                } else {
                    Some(LogLine::message(
                        tags,
                        Some("Grep".into()),
                        "Searching".into(),
                    ))
                }
            }
            "WebSearch" => {
                if let Some(input) = item.get("input")
                    && let Some(query) = input.get("query").and_then(|q| q.as_str())
                {
                    Some(LogLine::message(
                        tags,
                        Some("WebSearch".into()),
                        format!("Web search: {query}"),
                    ))
                } else {
                    Some(LogLine::message(
                        tags,
                        Some("WebSearch".into()),
                        "Web search".into(),
                    ))
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
                Some(LogLine::message(
                    tags,
                    Some(tool_name.into()),
                    format!("Using {tool_name}{description}"),
                ))
            }
        }
    }

    /// Formats a todo list update as a structured Todo LogLine
    fn format_todo_update(
        &self,
        todos: &[ClaudeTodoItem],
        parent_tool_use_id: Option<&str>,
    ) -> LogLine {
        let tags = self.build_tags(None, None, parent_tool_use_id);

        let items: Vec<TodoItem> = todos
            .iter()
            .map(|t| {
                let status = match t.status.as_str() {
                    "in_progress" => TodoStatus::InProgress,
                    "completed" => TodoStatus::Completed,
                    _ => TodoStatus::Pending,
                };
                TodoItem {
                    content: t.content.clone(),
                    status,
                    active_form: t.active_form.clone(),
                    priority: t.priority.clone(),
                }
            })
            .collect();

        LogLine::todo(tags, items)
    }

    /// Formats a result message and stores the task result
    fn format_result_message(&mut self, msg: ClaudeMessage) -> Option<LogLine> {
        if let Some(subtype) = &msg.subtype {
            let success = subtype == "success" && msg.is_error != Some(true);
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

            Some(LogLine::summary(
                success,
                message,
                cost_usd,
                msg.duration_ms,
                msg.num_turns,
            ))
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
    fn process_line(&mut self, line: &str) -> Option<LogLine> {
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
                        Some(LogLine::warning(vec![], None, "parsing error".into()))
                    }
                } else {
                    // Before JSON mode is active, pass through non-JSON lines as-is
                    // Don't set last_line_was_parse_error since we're not in JSON mode yet
                    Some(LogLine::message(vec![], None, line.to_string()))
                }
            }
        }
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

    /// Assert the log line is a Message variant with specific tool
    fn assert_message_tool(line: &LogLine, expected_tool: Option<&str>) {
        if let LogLine::Message { tool, .. } = line {
            assert_eq!(tool.as_deref(), expected_tool);
        } else {
            panic!("Expected Message variant, got {:?}", line);
        }
    }

    /// Assert the log line contains a tag
    fn has_tag(line: &LogLine, tag: &str) {
        let tags = match line {
            LogLine::Message { tags, .. } => tags,
            LogLine::Todo { tags, .. } => tags,
            _ => panic!("Variant has no tags"),
        };
        assert!(
            tags.iter().any(|t| t == tag),
            "Expected tag '{}' in {:?}",
            tag,
            tags
        );
    }

    /// Assert the log line does NOT contain a tag
    fn no_tag(line: &LogLine, tag: &str) {
        let tags = match line {
            LogLine::Message { tags, .. } => tags,
            LogLine::Todo { tags, .. } => tags,
            LogLine::Summary { .. } => return, // Summary has no tags
        };
        assert!(
            !tags.iter().any(|t| t == tag),
            "Did not expect tag '{}' in {:?}",
            tag,
            tags
        );
    }

    fn get_message_text(line: &LogLine) -> &str {
        match line {
            LogLine::Message { message, .. } => message,
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_todo_processing() {
        let mut processor = ClaudeLogProcessor::new();

        // Test basic format with structured todo items
        let todos = r#"[
            {"content": "Analyze current usage", "status": "in_progress", "activeForm": "Analyzing current usage"},
            {"content": "Move file to new location", "status": "pending", "activeForm": "Moving file to new location"},
            {"content": "Update imports", "status": "completed", "activeForm": "Updating imports"}
        ]"#;
        let msg = todo_msg(todos);
        let output = processor.process_line(&msg).unwrap();
        if let LogLine::Todo { items, .. } = &output {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].status, TodoStatus::InProgress);
            assert_eq!(
                items[0].active_form.as_deref(),
                Some("Analyzing current usage")
            );
            assert_eq!(items[1].status, TodoStatus::Pending);
            assert_eq!(items[2].status, TodoStatus::Completed);
        } else {
            panic!("Expected Todo variant");
        }

        // Test priority is preserved
        let todos_with_priority = r#"[
            {"content": "High priority task", "status": "pending", "activeForm": "Working on high priority", "priority": "high"},
            {"content": "Low priority task", "status": "completed", "activeForm": "Completed low priority", "priority": "low"}
        ]"#;
        let msg = todo_msg(todos_with_priority);
        let output = processor.process_line(&msg).unwrap();
        if let LogLine::Todo { items, .. } = &output {
            assert_eq!(items[0].priority.as_deref(), Some("high"));
            assert_eq!(items[1].priority.as_deref(), Some("low"));
        } else {
            panic!("Expected Todo variant");
        }
    }

    #[test]
    fn test_user_message_with_todo_result() {
        let mut processor = ClaudeLogProcessor::new();
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
        let line = result.unwrap();
        assert_message_tool(&line, Some("TodoWrite"));
        assert!(get_message_text(&line).contains("result - 2 todos"));
    }

    #[test]
    fn test_assistant_message_formats() {
        let mut processor = ClaudeLogProcessor::new();

        // Text message without model
        let json = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello, world!"}]}}"#;
        let output = processor.process_line(json).unwrap();
        assert_eq!(get_message_text(&output), "Hello, world!");

        // Text message with model
        let json = r#"{"type": "assistant", "message": {"model": "claude-opus", "content": [{"type": "text", "text": "Hello with model!"}]}}"#;
        let output = processor.process_line(json).unwrap();
        assert_eq!(get_message_text(&output), "Hello with model!");
        has_tag(&output, "claude-opus");

        // Direct string content with model
        let json = r#"{"type": "assistant", "message": {"model": "claude-3-sonnet", "content": "Direct string content"}}"#;
        let output = processor.process_line(json).unwrap();
        assert_eq!(get_message_text(&output), "Direct string content");
        has_tag(&output, "claude-3-sonnet");

        // Bash tool use
        let json = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Bash", "input": {"command": "cargo test"}}]}}"#;
        let output = processor.process_line(json).unwrap();
        assert!(get_message_text(&output).contains("Running: cargo test"));
    }

    #[test]
    fn test_result_and_summary_messages() {
        let mut processor = ClaudeLogProcessor::new();

        // Test result message with cost and duration
        let result_json = r#"{
            "type": "result",
            "subtype": "success",
            "cost_usd": 0.15,
            "duration_ms": 45000,
            "result": "Task completed successfully"
        }"#;
        let output = processor.process_line(result_json).unwrap();
        if let LogLine::Summary {
            success,
            cost_usd,
            duration_ms,
            ..
        } = &output
        {
            assert!(success);
            assert_eq!(*cost_usd, Some(0.15));
            assert_eq!(*duration_ms, Some(45000));
        } else {
            panic!("Expected Summary variant");
        }

        let final_result = processor.get_final_result().unwrap();
        assert!(final_result.success);
        assert_eq!(final_result.cost_usd, Some(0.15));

        // Test result with subtype "success" but is_error true (e.g., auth failure)
        let mut processor = ClaudeLogProcessor::new();
        let error_result_json = r#"{
            "type": "result",
            "subtype": "success",
            "is_error": true,
            "result": "Failed to authenticate. API Error: 401 Unauthorized"
        }"#;
        let output = processor.process_line(error_result_json).unwrap();
        if let LogLine::Summary { success, .. } = &output {
            assert!(!success);
        } else {
            panic!("Expected Summary variant");
        }

        let final_result = processor.get_final_result().unwrap();
        assert!(!final_result.success);
        assert_eq!(
            final_result.message,
            "Failed to authenticate. API Error: 401 Unauthorized"
        );

        // Test summary message (not the result type)
        let mut processor = ClaudeLogProcessor::new();
        let summary_json = r#"{
            "type": "summary",
            "summary": "Refactoring completed: Renamed XdgDirectories to TskConfig"
        }"#;
        let output = processor.process_line(summary_json).unwrap();
        if let LogLine::Message { level, message, .. } = &output {
            assert_eq!(*level, Level::Success);
            assert_eq!(
                message,
                "Refactoring completed: Renamed XdgDirectories to TskConfig"
            );
        } else {
            panic!("Expected Message variant with Success level");
        }
    }

    #[test]
    fn test_json_mode_behavior() {
        let mut processor = ClaudeLogProcessor::new();

        // Initially not in JSON mode
        assert!(!processor.json_mode_active);

        // Before JSON mode is active, non-JSON lines should be passed through as-is
        let result = processor.process_line("Error: Claude Code is misconfigured");
        let line = result.unwrap();
        assert_eq!(
            get_message_text(&line),
            "Error: Claude Code is misconfigured"
        );
        assert!(!processor.json_mode_active);

        let result = processor.process_line("Configuration warning: API key missing");
        let line = result.unwrap();
        assert_eq!(
            get_message_text(&line),
            "Configuration warning: API key missing"
        );
        assert!(!processor.json_mode_active);

        // First valid JSON line activates JSON mode
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello"}]}}"#;
        let result = processor.process_line(json);
        let line = result.unwrap();
        assert_eq!(get_message_text(&line), "Hello");
        assert!(processor.json_mode_active);

        // After JSON mode is active, non-JSON lines should show parsing error
        let result = processor.process_line("This is not JSON");
        let line = result.unwrap();
        if let LogLine::Message { level, message, .. } = &line {
            assert_eq!(*level, Level::Warning);
            assert_eq!(message, "parsing error");
        } else {
            panic!("Expected Message variant");
        }

        // Consecutive parsing errors should be suppressed
        let result = processor.process_line("random text");
        assert!(result.is_none());
    }

    #[test]
    fn test_empty_tool_results_filtered() {
        let mut processor = ClaudeLogProcessor::new();

        // Test empty tool result (should return None)
        let empty_tool_result_json = r#"{
            "type": "user",
            "message": {
                "role": "user",
                "content": [{"tool_use_id": "toolu_456", "type": "tool_result", "content": ""}]
            }
        }"#;

        let result = processor.process_line(empty_tool_result_json);
        assert!(result.is_none());
    }

    #[test]
    fn test_sub_agent_tagging_scenarios() {
        let mut processor = ClaudeLogProcessor::new();

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
        let mut processor = ClaudeLogProcessor::new();

        // Test with prompt
        let msg1 = task_msg(
            "toolu_123",
            "software-architect",
            "Analyze code",
            Some("Check patterns"),
        );
        let output = processor.process_line(&msg1).unwrap();
        let text = get_message_text(&output);
        assert!(text.contains("Starting: Analyze code (software-architect)"));
        assert!(text.contains("Prompt: Check patterns"));

        let result = task_result("toolu_123", "Analysis complete", None);
        let output = processor.process_line(&result).unwrap();
        let text = get_message_text(&output);
        assert!(text.contains("Complete: Analyze code (software-architect)"));

        // Test array content with sub-agent output
        let mut processor = ClaudeLogProcessor::new();
        let msg2 = task_msg("toolu_456", "data-analyst", "Analyze patterns", None);
        processor.process_line(&msg2);

        let result = task_result(
            "toolu_456",
            r#"[{"type": "text", "text": "Found 5 patterns. Pattern A: 45%."}]"#,
            None,
        );
        let output = processor.process_line(&result).unwrap();
        let text = get_message_text(&output);
        assert!(text.contains("Complete: Analyze patterns (data-analyst)"));
        assert!(text.contains("Sub-agent Output:"));
        assert!(text.contains("Found 5 patterns"));

        // Test empty content (no sub-agent output section)
        let mut processor = ClaudeLogProcessor::new();
        let msg3 = task_msg("toolu_789", "test-agent", "Test edge cases", None);
        processor.process_line(&msg3);

        let result = task_result("toolu_789", "", None);
        let output = processor.process_line(&result).unwrap();
        let text = get_message_text(&output);
        assert!(text.contains("Complete: Test edge cases (test-agent)"));
        assert!(!text.contains("Sub-agent Output:"));
    }

    #[test]
    fn test_parse_error_deduplication() {
        let mut processor = ClaudeLogProcessor::new();

        // First, activate JSON mode with a valid JSON line
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello"}]}}"#;
        let result = processor.process_line(json);
        assert_eq!(get_message_text(&result.unwrap()), "Hello");
        assert!(processor.json_mode_active);

        // First parsing error should be shown
        let result = processor.process_line("This is not JSON").unwrap();
        if let LogLine::Message { level, message, .. } = &result {
            assert_eq!(*level, Level::Warning);
            assert_eq!(message, "parsing error");
        }

        // Second parsing error should be suppressed (returns None)
        assert!(processor.process_line("Another non-JSON line").is_none());

        // Third parsing error should also be suppressed
        assert!(
            processor
                .process_line("Yet another non-JSON line")
                .is_none()
        );
    }

    #[test]
    fn test_parse_error_resume_after_valid_json() {
        let mut processor = ClaudeLogProcessor::new();

        // Activate JSON mode
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "First"}]}}"#;
        processor.process_line(json);

        // First parsing error shown
        let result = processor.process_line("Bad line 1").unwrap();
        if let LogLine::Message { level, .. } = &result {
            assert_eq!(*level, Level::Warning);
        }

        // Consecutive errors suppressed
        assert!(processor.process_line("Bad line 2").is_none());
        assert!(processor.process_line("Bad line 3").is_none());

        // Valid JSON resets the error flag
        let json = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Second"}]}}"#;
        let result = processor.process_line(json);
        assert_eq!(get_message_text(&result.unwrap()), "Second");

        // Next parsing error should be shown again
        let result = processor.process_line("Bad line 4").unwrap();
        if let LogLine::Message { level, .. } = &result {
            assert_eq!(*level, Level::Warning);
        }

        // And consecutive errors suppressed again
        assert!(processor.process_line("Bad line 5").is_none());
    }

    #[test]
    fn test_empty_lines_dont_reset_error_tracking() {
        let mut processor = ClaudeLogProcessor::new();

        // Activate JSON mode
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Start"}]}}"#;
        processor.process_line(json);

        // First parsing error shown
        let result = processor.process_line("Bad line 1").unwrap();
        if let LogLine::Message { level, .. } = &result {
            assert_eq!(*level, Level::Warning);
        }

        // Empty line should not reset error tracking
        assert!(processor.process_line("").is_none());

        // Next parsing error should still be suppressed
        assert!(processor.process_line("Bad line 2").is_none());

        // Another empty line
        assert!(processor.process_line("   ").is_none());

        // Parsing error still suppressed
        assert!(processor.process_line("Bad line 3").is_none());

        // Valid JSON resets the state
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Good"}]}}"#;
        processor.process_line(json);

        // Empty line after valid JSON
        assert!(processor.process_line("").is_none());

        // Next parsing error should be shown (error flag was reset by valid JSON)
        let result = processor.process_line("Bad line 4").unwrap();
        if let LogLine::Message { level, .. } = &result {
            assert_eq!(*level, Level::Warning);
        }
    }

    #[test]
    fn test_pre_json_mode_behavior_unchanged() {
        let mut processor = ClaudeLogProcessor::new();

        // Before JSON mode, non-JSON lines should be passed through
        let result = processor.process_line("Configuration error message");
        assert_eq!(
            get_message_text(&result.unwrap()),
            "Configuration error message"
        );
        assert!(!processor.json_mode_active);

        // Multiple non-JSON lines should all be passed through
        let result = processor.process_line("Warning: API key missing");
        assert_eq!(
            get_message_text(&result.unwrap()),
            "Warning: API key missing"
        );
        assert!(!processor.json_mode_active);

        let result = processor.process_line("Another warning");
        assert_eq!(get_message_text(&result.unwrap()), "Another warning");
        assert!(!processor.json_mode_active);

        // Now activate JSON mode
        let json =
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Start"}]}}"#;
        let result = processor.process_line(json);
        assert_eq!(get_message_text(&result.unwrap()), "Start");
        assert!(processor.json_mode_active);

        // Now parsing errors should be shown and deduplicated
        let result = processor.process_line("Bad line 1").unwrap();
        if let LogLine::Message { level, .. } = &result {
            assert_eq!(*level, Level::Warning);
        }

        assert!(processor.process_line("Bad line 2").is_none());
    }
}
