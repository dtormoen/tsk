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
    /// Lines of log content for the selected task
    pub log_content: Vec<String>,
    /// Vertical scroll offset for the log viewer
    pub log_scroll: u16,
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
            server_messages: Vec::new(),
            workers_active: 0,
            workers_total,
            should_quit: false,
            task_panel_width: 0,
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

    /// Move the task list selection up, clamping at the first item
    pub fn select_previous_task(&mut self) {
        if self.tasks.is_empty() {
            return;
        }
        let current = self.task_list_state.selected().unwrap_or(0);
        let prev = current.saturating_sub(1);
        self.task_list_state.select(Some(prev));
    }

    /// Scroll the log viewer up by the given amount, clamping at 0
    pub fn scroll_logs_up(&mut self, amount: u16) {
        self.log_scroll = self.log_scroll.saturating_sub(amount);
    }

    /// Scroll the log viewer down by the given amount
    pub fn scroll_logs_down(&mut self, amount: u16) {
        self.log_scroll = self.log_scroll.saturating_add(amount);
    }

    /// Load the agent log file for the currently selected task, resetting scroll to the top.
    ///
    /// Use this when the user explicitly changes task selection.
    /// Reads from `{data_dir}/tasks/{task_id}/output/agent.log`.
    pub fn load_logs_for_selected_task(&mut self, data_dir: &Path) {
        self.log_scroll = 0;
        self.reload_logs(data_dir);
    }

    /// Refresh the log file content for the currently selected task without resetting scroll.
    ///
    /// Use this for periodic live-tailing refreshes that should not disrupt the
    /// user's scroll position.
    pub fn refresh_logs(&mut self, data_dir: &Path) {
        self.reload_logs(data_dir);
    }

    /// Internal helper that reads the agent.log for the selected task.
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
                self.log_content = content.lines().map(String::from).collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use std::fs;

    #[test]
    fn test_new_defaults() {
        let app = TuiApp::new(4);

        assert_eq!(app.focus, Panel::Tasks);
        assert_eq!(app.workers_total, 4);
        assert_eq!(app.workers_active, 0);
        assert!(!app.should_quit);
        assert!(app.server_messages.is_empty());
        assert!(app.tasks.is_empty());

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
        fs::write(log_dir.join("agent.log"), "line 1\nline 2\nline 3\n").unwrap();

        let mut app = TuiApp::new(1);
        app.tasks = vec![Task {
            id: task_id.to_string(),
            name: "test-task".to_string(),
            branch_name: "tsk/feat/test-task/test-task-123".to_string(),
            ..Task::test_default()
        }];

        // Set some scroll offset to verify it resets
        app.log_scroll = 10;

        app.load_logs_for_selected_task(data_dir);

        assert_eq!(app.log_content, vec!["line 1", "line 2", "line 3"]);
        assert_eq!(app.log_scroll, 0);

        // Loading with no selection clears content
        app.task_list_state.select(None);
        app.load_logs_for_selected_task(data_dir);
        assert!(app.log_content.is_empty());
    }
}
