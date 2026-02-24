use super::app::{Panel, TuiApp};
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use std::path::Path;

/// Handle a crossterm event, updating TUI application state accordingly.
///
/// Processes keyboard navigation (vim-style and arrow keys) for panel
/// switching, task selection, and log scrolling. Also handles mouse
/// scroll events, routing to the appropriate panel based on cursor position.
pub fn handle_event(app: &mut TuiApp, event: &Event, data_dir: &Path) {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') => {
                app.should_quit = true;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                app.focus = Panel::Tasks;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                app.focus = Panel::Logs;
            }
            KeyCode::Char('j') | KeyCode::Down => match app.focus {
                Panel::Tasks => {
                    app.select_next_task();
                    app.load_logs_for_selected_task(data_dir);
                }
                Panel::Logs => {
                    app.scroll_logs_down(1);
                }
            },
            KeyCode::Char('k') | KeyCode::Up => match app.focus {
                Panel::Tasks => {
                    app.select_previous_task();
                    app.load_logs_for_selected_task(data_dir);
                }
                Panel::Logs => {
                    app.scroll_logs_up(1);
                }
            },
            KeyCode::PageDown if app.focus == Panel::Logs => {
                app.scroll_logs_down(20);
            }
            KeyCode::PageUp if app.focus == Panel::Logs => {
                app.scroll_logs_up(20);
            }
            _ => {}
        },
        Event::Mouse(mouse) => {
            let in_task_panel = mouse.column < app.task_panel_width;

            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    if in_task_panel {
                        app.select_next_task();
                        app.load_logs_for_selected_task(data_dir);
                    } else {
                        app.scroll_logs_down(3);
                    }
                }
                MouseEventKind::ScrollUp => {
                    if in_task_panel {
                        app.select_previous_task();
                        app.load_logs_for_selected_task(data_dir);
                    } else {
                        app.scroll_logs_up(3);
                    }
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if in_task_panel {
                        app.focus = Panel::Tasks;
                        if let Some(inner_row) = mouse.row.checked_sub(app.task_list_top) {
                            let task_index = inner_row as usize / 2 + app.task_list_state.offset();
                            if task_index < app.tasks.len() {
                                app.select_task(task_index);
                                app.load_logs_for_selected_task(data_dir);
                            }
                        }
                    } else {
                        app.focus = Panel::Logs;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use std::fs;

    fn make_key_event(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn make_mouse_scroll(kind: MouseEventKind, column: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row: 5,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn make_mouse_click(column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn app_with_tasks() -> TuiApp {
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
        app
    }

    #[test]
    fn test_quit() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = TuiApp::new(1);
        assert!(!app.should_quit);

        handle_event(&mut app, &make_key_event(KeyCode::Char('q')), tmp.path());
        assert!(app.should_quit);
    }

    #[test]
    fn test_panel_switching() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = TuiApp::new(1);
        assert_eq!(app.focus, Panel::Tasks);

        handle_event(&mut app, &make_key_event(KeyCode::Char('l')), tmp.path());
        assert_eq!(app.focus, Panel::Logs);

        handle_event(&mut app, &make_key_event(KeyCode::Char('h')), tmp.path());
        assert_eq!(app.focus, Panel::Tasks);

        handle_event(&mut app, &make_key_event(KeyCode::Right), tmp.path());
        assert_eq!(app.focus, Panel::Logs);

        handle_event(&mut app, &make_key_event(KeyCode::Left), tmp.path());
        assert_eq!(app.focus, Panel::Tasks);
    }

    #[test]
    fn test_task_navigation_with_j_k() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with_tasks();

        assert_eq!(app.task_list_state.selected(), Some(0));

        handle_event(&mut app, &make_key_event(KeyCode::Char('j')), tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(1));

        handle_event(&mut app, &make_key_event(KeyCode::Char('k')), tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(0));
    }

    #[test]
    fn test_task_navigation_with_arrows() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with_tasks();

        handle_event(&mut app, &make_key_event(KeyCode::Down), tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(1));

        handle_event(&mut app, &make_key_event(KeyCode::Up), tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(0));
    }

    #[test]
    fn test_log_scrolling() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = TuiApp::new(1);
        app.focus = Panel::Logs;
        app.log_content = (0..100)
            .map(|i| crate::agent::log_line::LogLine::message(vec![], None, format!("line {i}")))
            .collect();
        app.log_viewport_height = 10;
        app.log_wrapped_line_count = 100;

        handle_event(&mut app, &make_key_event(KeyCode::Char('j')), tmp.path());
        assert_eq!(app.log_scroll, 1);

        handle_event(&mut app, &make_key_event(KeyCode::Char('k')), tmp.path());
        assert_eq!(app.log_scroll, 0);

        handle_event(&mut app, &make_key_event(KeyCode::PageDown), tmp.path());
        assert_eq!(app.log_scroll, 20);

        handle_event(&mut app, &make_key_event(KeyCode::PageUp), tmp.path());
        assert_eq!(app.log_scroll, 0);
    }

    #[test]
    fn test_task_selection_loads_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();

        // Create log files for tasks in JSON-lines format
        let log_dir = data_dir.join("tasks").join("t2").join("output");
        fs::create_dir_all(&log_dir).unwrap();
        let log_line = crate::agent::log_line::LogLine::message(vec![], None, "log from t2".into());
        let json = serde_json::to_string(&log_line).unwrap();
        fs::write(log_dir.join("agent.log"), format!("{json}\n")).unwrap();

        let mut app = app_with_tasks();

        handle_event(&mut app, &make_key_event(KeyCode::Char('j')), data_dir);
        assert_eq!(app.task_list_state.selected(), Some(1));
        assert_eq!(app.log_content.len(), 1);
        assert_eq!(
            app.log_content,
            vec![crate::agent::log_line::LogLine::message(
                vec![],
                None,
                "log from t2".into()
            )]
        );
    }

    #[test]
    fn test_mouse_scroll_in_task_panel() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with_tasks();
        app.task_panel_width = 30;

        // Column 0 is within the task panel
        let scroll_down = make_mouse_scroll(MouseEventKind::ScrollDown, 0);
        handle_event(&mut app, &scroll_down, tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(1));

        let scroll_up = make_mouse_scroll(MouseEventKind::ScrollUp, 0);
        handle_event(&mut app, &scroll_up, tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(0));
    }

    #[test]
    fn test_mouse_scroll_in_log_panel() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = TuiApp::new(1);
        app.task_panel_width = 30;
        app.log_content = (0..100)
            .map(|i| crate::agent::log_line::LogLine::message(vec![], None, format!("line {i}")))
            .collect();
        app.log_viewport_height = 10;
        app.log_wrapped_line_count = 100;

        // Column 79 is well past the task panel boundary
        let scroll_down = make_mouse_scroll(MouseEventKind::ScrollDown, 79);
        handle_event(&mut app, &scroll_down, tmp.path());
        assert_eq!(app.log_scroll, 3);

        let scroll_up = make_mouse_scroll(MouseEventKind::ScrollUp, 79);
        handle_event(&mut app, &scroll_up, tmp.path());
        assert_eq!(app.log_scroll, 0);
    }

    #[test]
    fn test_page_keys_only_work_in_logs_panel() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = TuiApp::new(1);
        app.focus = Panel::Tasks;

        // PageDown/PageUp should be ignored when tasks panel is focused
        handle_event(&mut app, &make_key_event(KeyCode::PageDown), tmp.path());
        assert_eq!(app.log_scroll, 0);

        handle_event(&mut app, &make_key_event(KeyCode::PageUp), tmp.path());
        assert_eq!(app.log_scroll, 0);
    }

    #[test]
    fn test_mouse_click_selects_task() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with_tasks();
        app.task_panel_width = 30;
        app.task_list_top = 2; // header(1) + top border(1)

        // Click on the second task (rows 4-5, each task is 2 rows tall)
        handle_event(&mut app, &make_mouse_click(5, 4), tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(1));
        assert_eq!(app.focus, Panel::Tasks);

        // Click on the third task (rows 6-7)
        handle_event(&mut app, &make_mouse_click(5, 6), tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(2));
    }

    #[test]
    fn test_mouse_click_loads_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();

        let log_dir = data_dir.join("tasks").join("t2").join("output");
        fs::create_dir_all(&log_dir).unwrap();
        let log_line = crate::agent::log_line::LogLine::message(vec![], None, "log from t2".into());
        let json = serde_json::to_string(&log_line).unwrap();
        fs::write(log_dir.join("agent.log"), format!("{json}\n")).unwrap();

        let mut app = app_with_tasks();
        app.task_panel_width = 30;
        app.task_list_top = 2;

        // Click on the second task
        handle_event(&mut app, &make_mouse_click(5, 4), data_dir);
        assert_eq!(app.task_list_state.selected(), Some(1));
        assert_eq!(app.log_content.len(), 1);
        assert_eq!(
            app.log_content,
            vec![crate::agent::log_line::LogLine::message(
                vec![],
                None,
                "log from t2".into()
            )]
        );
    }

    #[test]
    fn test_mouse_click_out_of_bounds_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with_tasks();
        app.task_panel_width = 30;
        app.task_list_top = 2;

        // Click well below the last task (row 20, only 3 tasks at rows 2-7)
        handle_event(&mut app, &make_mouse_click(5, 20), tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(0)); // unchanged

        // Click on the top border (row 1, task_list_top is 2)
        handle_event(&mut app, &make_mouse_click(5, 1), tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(0)); // unchanged
    }

    #[test]
    fn test_mouse_click_empty_task_list() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = TuiApp::new(1);
        app.task_panel_width = 30;
        app.task_list_top = 2;

        // Click in the task area with no tasks — selection should not change
        handle_event(&mut app, &make_mouse_click(5, 3), tmp.path());
        assert_eq!(app.task_list_state.selected(), Some(0));
    }

    #[test]
    fn test_mouse_click_focuses_panel() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_with_tasks();
        app.task_panel_width = 30;
        app.task_list_top = 2;

        // Click on log panel (column 50 > task_panel_width 30)
        handle_event(&mut app, &make_mouse_click(50, 5), tmp.path());
        assert_eq!(app.focus, Panel::Logs);

        // Click on task panel focuses it back
        handle_event(&mut app, &make_mouse_click(5, 2), tmp.path());
        assert_eq!(app.focus, Panel::Tasks);
    }

    #[test]
    fn test_release_key_events_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = TuiApp::new(1);

        let release_event = Event::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
        handle_event(&mut app, &release_event, tmp.path());
        assert!(!app.should_quit);
    }
}
