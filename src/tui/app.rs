use crate::agent::log_line::LogLine;
use crate::task::Task;
use ratatui::widgets::ListState;
use std::path::Path;

/// Identifies which panel currently has focus in the TUI
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Panel {
    #[default]
    Tasks,
    Logs,
}

/// Application state for the TUI server dashboard.
///
/// Holds the current UI state including task list, log viewer, and
/// server status information. This struct is updated in response to
/// user input and server events.
pub struct TuiApp {
    /// Which panel currently has focus
    pub focus: Panel,
    /// Selection state for the task list
    pub task_list_state: ListState,
    /// Current list of tasks from storage
    pub tasks: Vec<Task>,
    /// Structured log lines for the selected task
    pub log_content: Vec<LogLine>,
    /// Vertical scroll offset for the log viewer (in visual lines)
    pub log_scroll: usize,
    /// Height of the log viewer viewport in rows (set during render)
    pub log_viewport_height: usize,
    /// Whether the log viewer auto-scrolls to follow new content
    pub log_follow: bool,
    /// Timestamped server messages (status, warnings, events)
    pub server_messages: Vec<(chrono::DateTime<chrono::Local>, String)>,
    /// Number of currently active workers
    pub workers_active: usize,
    /// Total number of configured workers
    pub workers_total: usize,
    /// Whether the application should exit
    pub should_quit: bool,
    /// Computed width of the task panel (set during render)
    pub task_panel_width: u16,
    /// Y coordinate of the first task row in the task list (set during render)
    pub task_list_top: u16,
}

impl TuiApp {
    /// Create a new TUI application state with the given worker count
    pub fn new(workers_total: usize) -> Self {
        let mut task_list_state = ListState::default();
        task_list_state.select(Some(0));

        Self {
            focus: Panel::default(),
            task_list_state,
            tasks: Vec::new(),
            log_content: Vec::new(),
            log_scroll: 0,
            log_viewport_height: 0,
            log_follow: true,
            server_messages: Vec::new(),
            workers_active: 0,
            workers_total,
            should_quit: false,
            task_panel_width: 0,
            task_list_top: 0,
        }
    }

    /// Move the task list selection down, clamping at the last item
    pub fn select_next_task(&mut self) {
        if self.tasks.is_empty() {
            return;
        }
        let current = self.task_list_state.selected().unwrap_or(0);
        let next = (current + 1).min(self.tasks.len() - 1);
        self.task_list_state.select(Some(next));
    }

    /// Select a task by index, ignoring out-of-bounds values
    pub fn select_task(&mut self, index: usize) {
        if index < self.tasks.len() {
            self.task_list_state.select(Some(index));
        }
    }

    /// Move the task list selection up, clamping at the first item
    pub fn select_previous_task(&mut self) {
        if self.tasks.is_empty() {
            return;
        }
        let current = self.task_list_state.selected().unwrap_or(0);
        let prev = current.saturating_sub(1);
        self.task_list_state.select(Some(prev));
    }

    /// Count the total number of visual lines across all log entries.
    ///
    /// Each LogLine maps to visual lines as follows:
    /// - Message: 1 line per newline-separated segment
    /// - Todo: 1 line per item + 1 summary line
    /// - Summary: 1 line
    pub fn visual_line_count(&self) -> usize {
        self.log_content.iter().map(visual_lines_for).sum()
    }

    /// Maximum scroll offset for the log viewer, based on content and viewport size
    pub fn max_log_scroll(&self) -> usize {
        self.visual_line_count()
            .saturating_sub(self.log_viewport_height)
    }

    /// Clamp the scroll offset so it does not exceed the maximum
    pub fn clamp_log_scroll(&mut self) {
        self.log_scroll = self.log_scroll.min(self.max_log_scroll());
    }

    /// Scroll the log viewer up by the given amount, clamping at 0
    pub fn scroll_logs_up(&mut self, amount: usize) {
        self.log_scroll = self.log_scroll.saturating_sub(amount);
        self.log_follow = self.log_scroll >= self.max_log_scroll();
    }

    /// Scroll the log viewer down by the given amount, clamping at the bottom
    pub fn scroll_logs_down(&mut self, amount: usize) {
        self.log_scroll = self.log_scroll.saturating_add(amount);
        self.clamp_log_scroll();
        self.log_follow = self.log_scroll >= self.max_log_scroll();
    }

    /// Load the agent log file for the currently selected task, scrolling to the bottom.
    ///
    /// Use this when the user explicitly changes task selection.
    /// Reads from `{data_dir}/tasks/{task_id}/output/agent.log`.
    pub fn load_logs_for_selected_task(&mut self, data_dir: &Path) {
        self.reload_logs(data_dir);
        self.log_scroll = self.max_log_scroll();
        self.log_follow = true;
    }

    /// Refresh the log file content for the currently selected task without resetting scroll.
    ///
    /// Use this for periodic live-tailing refreshes that should not disrupt the
    /// user's scroll position. When follow mode is active, auto-scrolls to the bottom.
    pub fn refresh_logs(&mut self, data_dir: &Path) {
        self.reload_logs(data_dir);
        if self.log_follow {
            self.log_scroll = self.max_log_scroll();
        }
        self.clamp_log_scroll();
    }

    /// Internal helper that reads the agent.log for the selected task.
    ///
    /// Parses JSON-lines format; non-JSON lines (pre-migration fallback) are
    /// wrapped as `LogLine::Message`.
    fn reload_logs(&mut self, data_dir: &Path) {
        let selected = match self.task_list_state.selected() {
            Some(idx) if idx < self.tasks.len() => idx,
            _ => {
                self.log_content.clear();
                return;
            }
        };

        let task = &self.tasks[selected];
        let log_path = data_dir
            .join("tasks")
            .join(&task.id)
            .join("output")
            .join("agent.log");

        match std::fs::read_to_string(&log_path) {
            Ok(content) => {
                self.log_content = content
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(|line| {
                        serde_json::from_str::<LogLine>(line).unwrap_or_else(|_| {
                            // Fallback for pre-migration plain text lines
                            LogLine::message(vec![], None, line.to_string())
                        })
                    })
                    .collect();
            }
            Err(_) => {
                self.log_content.clear();
            }
        }
    }

    /// Replace the task list with new data, preserving the current selection
    /// if the previously selected task still exists in the new list.
    pub fn update_tasks(&mut self, tasks: Vec<Task>) {
        let previously_selected_id = self
            .task_list_state
            .selected()
            .and_then(|idx| self.tasks.get(idx))
            .map(|t| t.id.clone());

        self.tasks = tasks;

        if let Some(prev_id) = previously_selected_id {
            let new_idx = self.tasks.iter().position(|t| t.id == prev_id);
            match new_idx {
                Some(idx) => self.task_list_state.select(Some(idx)),
                None if !self.tasks.is_empty() => self.task_list_state.select(Some(0)),
                None => self.task_list_state.select(None),
            }
        } else if !self.tasks.is_empty() {
            self.task_list_state.select(Some(0));
        }
    }
}

/// Count the number of visual lines a LogLine occupies when rendered.
pub fn visual_lines_for(log_line: &LogLine) -> usize {
    match log_line {
        LogLine::Message { message, .. } => {
            // Count newlines in message text
            message.lines().count().max(1)
        }
        LogLine::Todo { items, .. } => {
            // One line per item + summary count line
            items.len() + 1
        }
        LogLine::Summary { .. } => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::log_line::{LogLine, TodoItem, TodoStatus};
    use crate::task::Task;
    use std::fs;

    /// Helper to create LogLine entries for tests
    fn make_log_lines(count: usize) -> Vec<LogLine> {
        (0..count)
            .map(|i| LogLine::message(vec![], None, format!("line {i}")))
            .collect()
    }

    #[test]
    fn test_new_defaults() {
        let app = TuiApp::new(4);

        assert_eq!(app.focus, Panel::Tasks);
        assert_eq!(app.workers_total, 4);
        assert_eq!(app.workers_active, 0);
        assert!(!app.should_quit);
        assert!(app.server_messages.is_empty());
        assert!(app.tasks.is_empty());
        assert_eq!(app.log_viewport_height, 0);
        assert!(app.log_follow);

        // Panel::Logs variant is available for focus switching
        let mut app = app;
        app.focus = Panel::Logs;
        assert_eq!(app.focus, Panel::Logs);
    }

    #[test]
    fn test_select_next_previous_task() {
        let mut app = TuiApp::new(2);
        app.tasks = vec![
            Task {
                id: "t1".to_string(),
                name: "task-1".to_string(),
                branch_name: "tsk/feat/task-1/t1".to_string(),
                ..Task::test_default()
            },
            Task {
                id: "t2".to_string(),
                name: "task-2".to_string(),
                branch_name: "tsk/feat/task-2/t2".to_string(),
                ..Task::test_default()
            },
            Task {
                id: "t3".to_string(),
                name: "task-3".to_string(),
                branch_name: "tsk/feat/task-3/t3".to_string(),
                ..Task::test_default()
            },
        ];

        // Starts at 0
        assert_eq!(app.task_list_state.selected(), Some(0));

        // Next goes to 1
        app.select_next_task();
        assert_eq!(app.task_list_state.selected(), Some(1));

        // Next goes to 2
        app.select_next_task();
        assert_eq!(app.task_list_state.selected(), Some(2));

        // Next clamps at 2 (last item)
        app.select_next_task();
        assert_eq!(app.task_list_state.selected(), Some(2));

        // Previous goes to 1
        app.select_previous_task();
        assert_eq!(app.task_list_state.selected(), Some(1));

        // Go back to 0
        app.select_previous_task();
        assert_eq!(app.task_list_state.selected(), Some(0));

        // Previous clamps at 0 (first item)
        app.select_previous_task();
        assert_eq!(app.task_list_state.selected(), Some(0));
    }

    #[test]
    fn test_scroll_logs() {
        let mut app = TuiApp::new(1);
        // 20 lines of content, viewport of 10 -> max scroll = 10
        app.log_content = make_log_lines(20);
        app.log_viewport_height = 10;

        // Start at 0
        assert_eq!(app.log_scroll, 0);

        // Scroll down
        app.scroll_logs_down(5);
        assert_eq!(app.log_scroll, 5);

        // Scroll down more
        app.scroll_logs_down(3);
        assert_eq!(app.log_scroll, 8);

        // Scroll up partially
        app.scroll_logs_up(3);
        assert_eq!(app.log_scroll, 5);

        // Scroll up past 0 clamps at 0
        app.scroll_logs_up(10);
        assert_eq!(app.log_scroll, 0);

        // Scroll down past max clamps at max
        app.scroll_logs_down(100);
        assert_eq!(app.log_scroll, 10);
    }

    #[test]
    fn test_update_tasks_preserves_selection() {
        let mut app = TuiApp::new(1);

        let tasks_v1 = vec![
            Task {
                id: "t1".to_string(),
                name: "task-1".to_string(),
                branch_name: "tsk/feat/task-1/t1".to_string(),
                ..Task::test_default()
            },
            Task {
                id: "t2".to_string(),
                name: "task-2".to_string(),
                branch_name: "tsk/feat/task-2/t2".to_string(),
                ..Task::test_default()
            },
        ];
        app.update_tasks(tasks_v1);

        // Select second task
        app.task_list_state.select(Some(1));
        assert_eq!(app.tasks[1].id, "t2");

        // Update with reordered list (t2 moved to index 0, new t3 added)
        let tasks_v2 = vec![
            Task {
                id: "t2".to_string(),
                name: "task-2".to_string(),
                branch_name: "tsk/feat/task-2/t2".to_string(),
                ..Task::test_default()
            },
            Task {
                id: "t3".to_string(),
                name: "task-3".to_string(),
                branch_name: "tsk/feat/task-3/t3".to_string(),
                ..Task::test_default()
            },
        ];
        app.update_tasks(tasks_v2);

        // Selection should follow t2 to its new index (0)
        assert_eq!(app.task_list_state.selected(), Some(0));
        assert_eq!(app.tasks[0].id, "t2");
    }

    #[test]
    fn test_load_logs_for_selected_task() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let data_dir = tmp_dir.path();

        let task_id = "test-task-123";
        let log_dir = data_dir.join("tasks").join(task_id).join("output");
        fs::create_dir_all(&log_dir).unwrap();
        // Write JSON-lines format
        let lines = [
            serde_json::to_string(&LogLine::message(vec![], None, "line 1".into())).unwrap(),
            serde_json::to_string(&LogLine::message(vec![], None, "line 2".into())).unwrap(),
            serde_json::to_string(&LogLine::message(vec![], None, "line 3".into())).unwrap(),
        ];
        fs::write(log_dir.join("agent.log"), lines.join("\n") + "\n").unwrap();

        let mut app = TuiApp::new(1);
        app.log_viewport_height = 2;
        app.tasks = vec![Task {
            id: task_id.to_string(),
            name: "test-task".to_string(),
            branch_name: "tsk/feat/test-task/test-task-123".to_string(),
            ..Task::test_default()
        }];

        // Set a non-bottom scroll position to verify load overrides it
        app.log_scroll = 0;
        app.log_follow = false;

        app.load_logs_for_selected_task(data_dir);

        assert_eq!(app.log_content.len(), 3);
        // Scroll starts at bottom: 3 lines - 2 viewport = 1
        assert_eq!(app.log_scroll, 1);
        assert!(app.log_follow);

        // Loading with no selection clears content
        app.task_list_state.select(None);
        app.load_logs_for_selected_task(data_dir);
        assert!(app.log_content.is_empty());
    }

    #[test]
    fn test_follow_mode() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let data_dir = tmp_dir.path();

        let task_id = "follow-task";
        let log_dir = data_dir.join("tasks").join(task_id).join("output");
        fs::create_dir_all(&log_dir).unwrap();

        // Write 20 JSON-lines
        let lines: Vec<String> = (0..20)
            .map(|i| {
                serde_json::to_string(&LogLine::message(vec![], None, format!("line {i}"))).unwrap()
            })
            .collect();
        fs::write(log_dir.join("agent.log"), lines.join("\n")).unwrap();

        let mut app = TuiApp::new(1);
        app.log_viewport_height = 10;
        app.tasks = vec![Task {
            id: task_id.to_string(),
            name: "follow-task".to_string(),
            branch_name: "tsk/feat/follow-task/follow-task".to_string(),
            ..Task::test_default()
        }];

        // Load logs - should be at bottom with follow enabled
        app.load_logs_for_selected_task(data_dir);
        assert!(app.log_follow);
        assert_eq!(app.log_scroll, 10); // 20 lines - 10 viewport

        // Scroll up disables follow
        app.scroll_logs_up(5);
        assert!(!app.log_follow);
        assert_eq!(app.log_scroll, 5);

        // Scroll back to bottom re-enables follow
        app.scroll_logs_down(5);
        assert!(app.log_follow);
        assert_eq!(app.log_scroll, 10);

        // Scroll up again, then refresh - should NOT auto-scroll
        app.scroll_logs_up(3);
        assert!(!app.log_follow);
        let scroll_before = app.log_scroll;
        app.refresh_logs(data_dir);
        assert_eq!(app.log_scroll, scroll_before);

        // Re-enable follow and refresh with more content - should auto-scroll
        app.log_follow = true;
        let lines: Vec<String> = (0..30)
            .map(|i| {
                serde_json::to_string(&LogLine::message(vec![], None, format!("line {i}"))).unwrap()
            })
            .collect();
        fs::write(log_dir.join("agent.log"), lines.join("\n")).unwrap();
        app.refresh_logs(data_dir);
        assert_eq!(app.log_scroll, 20); // 30 lines - 10 viewport
    }

    #[test]
    fn test_scroll_clamping() {
        let mut app = TuiApp::new(1);
        // Viewport bigger than content
        app.log_content = make_log_lines(5);
        app.log_viewport_height = 10;

        assert_eq!(app.max_log_scroll(), 0);

        app.scroll_logs_down(100);
        assert_eq!(app.log_scroll, 0);
        assert!(app.log_follow);
    }

    #[test]
    fn test_max_log_scroll() {
        let mut app = TuiApp::new(1);

        // 20 lines, viewport 10
        app.log_content = make_log_lines(20);
        app.log_viewport_height = 10;
        assert_eq!(app.max_log_scroll(), 10);

        // 5 lines, viewport 10
        app.log_content = make_log_lines(5);
        assert_eq!(app.max_log_scroll(), 0);

        // 0 lines, viewport 10
        app.log_content.clear();
        assert_eq!(app.max_log_scroll(), 0);
    }

    #[test]
    fn test_visual_lines_message() {
        // Single line message
        let line = LogLine::message(vec![], None, "hello".into());
        assert_eq!(visual_lines_for(&line), 1);

        // Multi-line message
        let line = LogLine::message(vec![], None, "line1\nline2\nline3".into());
        assert_eq!(visual_lines_for(&line), 3);
    }

    #[test]
    fn test_visual_lines_todo() {
        let items = vec![
            TodoItem {
                content: "a".into(),
                status: TodoStatus::Pending,
                active_form: None,
                priority: None,
            },
            TodoItem {
                content: "b".into(),
                status: TodoStatus::Completed,
                active_form: None,
                priority: None,
            },
        ];
        let line = LogLine::todo(vec![], items);
        // 2 items + 1 summary = 3
        assert_eq!(visual_lines_for(&line), 3);
    }

    #[test]
    fn test_visual_lines_summary() {
        let line = LogLine::summary(true, "done".into(), None, None, None);
        assert_eq!(visual_lines_for(&line), 1);
    }

    #[test]
    fn test_reload_logs_json_lines() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let data_dir = tmp_dir.path();

        let task_id = "json-test";
        let log_dir = data_dir.join("tasks").join(task_id).join("output");
        fs::create_dir_all(&log_dir).unwrap();

        // Write mixed: JSON-lines and a plain text fallback
        let json_line = serde_json::to_string(&LogLine::message(
            vec![],
            Some("Bash".into()),
            "cargo test".into(),
        ))
        .unwrap();
        let content = format!("{json_line}\nplain text line\n");
        fs::write(log_dir.join("agent.log"), content).unwrap();

        let mut app = TuiApp::new(1);
        app.tasks = vec![Task {
            id: task_id.to_string(),
            name: "json-test".to_string(),
            branch_name: "tsk/feat/json-test/json-test".to_string(),
            ..Task::test_default()
        }];

        app.load_logs_for_selected_task(data_dir);

        assert_eq!(app.log_content.len(), 2);
        // First should be the parsed JSON line
        if let LogLine::Message { tool, message, .. } = &app.log_content[0] {
            assert_eq!(tool.as_deref(), Some("Bash"));
            assert_eq!(message, "cargo test");
        } else {
            panic!("Expected Message variant");
        }
        // Second should be the fallback plain text
        if let LogLine::Message { message, .. } = &app.log_content[1] {
            assert_eq!(message, "plain text line");
        } else {
            panic!("Expected Message variant");
        }
    }
}
