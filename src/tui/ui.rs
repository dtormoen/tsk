use chrono::Utc;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::agent::log_line::{Level, LogLine, TodoStatus};
use crate::task::TaskStatus;

use super::app::{Panel, TuiApp};

/// Render the TUI dashboard to the terminal frame.
///
/// Draws a header with server status, a two-panel main area (task list
/// and log viewer), and a footer with key binding hints.
pub fn render(app: &mut TuiApp, frame: &mut Frame) {
    let area = frame.area();

    // Overall vertical layout: header, main content, footer
    let outer = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    render_header(app, frame, outer[0]);
    render_main(app, frame, outer[1]);
    render_footer(frame, outer[2]);
}

/// Render the header line showing worker status and latest server message.
fn render_header(app: &TuiApp, frame: &mut Frame, area: ratatui::layout::Rect) {
    let worker_text = format!("Workers: {}/{}", app.workers_active, app.workers_total);
    let latest_message = app
        .server_messages
        .last()
        .map(|(_, msg)| format!(" {msg}"))
        .unwrap_or_default();

    let line = Line::from(vec![
        Span::raw("TSK Server"),
        Span::raw(" \u{2502} "),
        Span::styled(worker_text, Style::default().fg(Color::Cyan)),
        Span::raw(latest_message),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

/// Render the two-panel main content area with dynamic task panel width.
fn render_main(app: &mut TuiApp, frame: &mut Frame, area: ratatui::layout::Rect) {
    let min_width: u16 = 30;
    let max_width = (area.width * 40 / 100).max(min_width);

    // Calculate the widest task row
    let content_width = app
        .tasks
        .iter()
        .map(|task| {
            let is_waiting = !task.parent_ids.is_empty() && task.status == TaskStatus::Queued;
            let status_text = if is_waiting {
                "WAITING"
            } else {
                match task.status {
                    TaskStatus::Complete => "COMPLETE",
                    TaskStatus::Running => "RUNNING",
                    TaskStatus::Queued => "QUEUED",
                    TaskStatus::Failed => "FAILED",
                }
            };
            // First line: " X name  STATUS"
            let first_line_len = 1 + 1 + 1 + task.name.len() + 2 + status_text.len();
            // Second line: "   project · type duration"
            let duration = format_duration(task);
            let second_line_len =
                3 + task.project.len() + 3 + task.task_type.len() + 1 + duration.len();
            first_line_len.max(second_line_len)
        })
        .max()
        .unwrap_or(0) as u16;

    // Add border padding (2 for left+right borders) + 1 for inner right padding
    let desired_width = (content_width + 3).max(min_width).min(max_width);

    app.task_panel_width = desired_width;

    let panels =
        Layout::horizontal([Constraint::Length(desired_width), Constraint::Min(0)]).split(area);

    render_task_list(app, frame, panels[0]);
    render_log_viewer(app, frame, panels[1]);
}

/// Render the task list panel on the left side.
fn render_task_list(app: &mut TuiApp, frame: &mut Frame, area: ratatui::layout::Rect) {
    let focused = app.focus == Panel::Tasks;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    // Store the inner area top for mouse click hit-testing
    app.task_list_top = area.y + 1; // +1 for top border

    let block = Block::default()
        .title(" Tasks ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let items: Vec<ListItem> = app
        .tasks
        .iter()
        .map(|task| {
            let is_waiting = !task.parent_ids.is_empty() && task.status == TaskStatus::Queued;

            let (icon, color) = if is_waiting {
                ("\u{25ce}", Color::DarkGray)
            } else {
                match task.status {
                    TaskStatus::Complete => ("\u{2713}", Color::Green),
                    TaskStatus::Running => ("\u{25b8}", Color::Yellow),
                    TaskStatus::Queued => ("\u{25cb}", Color::Blue),
                    TaskStatus::Failed => ("\u{2717}", Color::Red),
                }
            };

            let status_text = if is_waiting {
                "WAITING"
            } else {
                match task.status {
                    TaskStatus::Complete => "COMPLETE",
                    TaskStatus::Running => "RUNNING",
                    TaskStatus::Queued => "QUEUED",
                    TaskStatus::Failed => "FAILED",
                }
            };

            let duration = format_duration(task);

            let first_line = Line::from(vec![
                Span::raw(" "),
                Span::styled(icon, Style::default().fg(color)),
                Span::raw(" "),
                Span::raw(&task.name),
                Span::raw("  "),
                Span::styled(status_text, Style::default().fg(color)),
            ]);

            let second_line = Line::from(vec![
                Span::raw("   "),
                Span::styled(
                    format!("{} \u{00b7} {} {}", task.project, task.task_type, duration),
                    Style::default().fg(Color::Rgb(140, 140, 140)),
                ),
            ]);

            ListItem::new(Text::from(vec![first_line, second_line]))
        })
        .collect();

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(list, area, &mut app.task_list_state);
}

/// Render the log viewer panel on the right side with styled LogLine rendering.
fn render_log_viewer(app: &mut TuiApp, frame: &mut Frame, area: ratatui::layout::Rect) {
    let focused = app.focus == Panel::Logs;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    // Track viewport height (excluding borders) for scroll clamping
    let inner = Block::default().borders(Borders::ALL).inner(area);
    app.log_viewport_height = inner.height as usize;

    let selected_task_name = app
        .task_list_state
        .selected()
        .and_then(|idx| app.tasks.get(idx))
        .map(|t| t.name.as_str())
        .unwrap_or("");

    let title = if selected_task_name.is_empty() {
        " Logs ".to_string()
    } else {
        format!(" Logs - {selected_task_name} ")
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    if app.log_content.is_empty() {
        app.log_wrapped_line_count = 0;
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "No logs available",
            Style::default().fg(Color::DarkGray),
        )))
        .block(block)
        .alignment(Alignment::Center);
        frame.render_widget(placeholder, area);
    } else {
        // Render LogLine entries into styled ratatui Lines
        let mut lines: Vec<Line> = Vec::new();
        for log_line in &app.log_content {
            render_log_line(log_line, &mut lines);
        }

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });

        // Compute accurate wrapped line count for scroll clamping
        app.log_wrapped_line_count = paragraph.line_count(inner.width);
        app.clamp_log_scroll();
        if app.log_follow {
            app.log_scroll = app.max_log_scroll();
        }

        // ratatui's Paragraph::scroll takes (u16, u16); clamp for logs > 65535 lines
        let paragraph = paragraph
            .block(block)
            .scroll((app.log_scroll.min(u16::MAX as usize) as u16, 0));
        frame.render_widget(paragraph, area);
    }
}

/// Render a single LogLine into one or more styled ratatui Lines.
///
/// All spans use owned strings, so the output lines have `'static` lifetime
/// and do not borrow from the input LogLine.
fn render_log_line(log_line: &LogLine, lines: &mut Vec<Line<'static>>) {
    match log_line {
        LogLine::Message {
            level,
            tags,
            tool,
            message,
        } => {
            let level_color = match level {
                Level::Info => Color::Reset,
                Level::Success => Color::Green,
                Level::Warning => Color::Yellow,
                Level::Error => Color::Red,
            };

            let mut prefix_spans: Vec<Span> = Vec::new();

            for tag in tags {
                prefix_spans.push(Span::styled(
                    format!("[{tag}]"),
                    Style::default().fg(Color::Rgb(100, 100, 100)),
                ));
            }
            if !tags.is_empty() {
                prefix_spans.push(Span::raw(" "));
            }

            if let Some(tool_name) = tool {
                prefix_spans.push(Span::styled(
                    format!("{tool_name}: "),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            }

            if message.is_empty() {
                lines.push(Line::from(prefix_spans));
            } else {
                for (i, msg_line) in message.lines().enumerate() {
                    let mut spans = Vec::new();
                    if i == 0 {
                        spans.extend(prefix_spans.clone());
                    }
                    spans.push(Span::styled(
                        msg_line.to_string(),
                        Style::default().fg(level_color),
                    ));
                    lines.push(Line::from(spans));
                }
            }
        }
        LogLine::Todo { tags, items } => {
            // Header line: [tags] TodoWrite:
            let mut header_spans: Vec<Span> = Vec::new();
            for tag in tags {
                header_spans.push(Span::styled(
                    format!("[{tag}]"),
                    Style::default().fg(Color::Rgb(100, 100, 100)),
                ));
            }
            if !tags.is_empty() {
                header_spans.push(Span::raw(" "));
            }
            header_spans.push(Span::styled(
                "TodoWrite:",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::from(header_spans));

            // Render each item as a checkbox line
            for item in items.iter() {
                let mut spans: Vec<Span> = Vec::new();

                match item.status {
                    TodoStatus::Completed => {
                        spans.push(Span::styled(
                            format!("[x] {}", item.content),
                            Style::default().fg(Color::Green),
                        ));
                    }
                    TodoStatus::InProgress => {
                        let text = item.active_form.as_deref().unwrap_or(&item.content);
                        spans.push(Span::styled(
                            format!("[~] {text}"),
                            Style::default().fg(Color::Yellow),
                        ));
                    }
                    TodoStatus::Pending => {
                        spans.push(Span::raw(format!("[ ] {}", item.content)));
                    }
                }

                lines.push(Line::from(spans));
            }

            // Summary count line
            let completed = items
                .iter()
                .filter(|i| i.status == TodoStatus::Completed)
                .count();
            lines.push(Line::from(Span::styled(
                format!("{}/{} done", completed, items.len()),
                Style::default().fg(Color::Rgb(100, 100, 100)),
            )));
        }
        LogLine::Summary {
            success,
            message,
            cost_usd,
            duration_ms,
            num_turns,
        } => {
            let color = if *success { Color::Green } else { Color::Red };
            let status = if *success { "SUCCESS" } else { "FAILED" };

            let mut parts = vec![format!("{status}: {message}")];
            if let Some(cost) = cost_usd {
                parts.push(format!("${cost:.2}"));
            }
            if let Some(ms) = duration_ms {
                let secs = ms / 1000;
                parts.push(format!("{secs}s"));
            }
            if let Some(turns) = num_turns {
                parts.push(format!("{turns} turns"));
            }

            lines.push(Line::from(Span::styled(
                parts.join(" | "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )));
        }
    }
}

/// Render the footer line with key binding hints.
fn render_footer(frame: &mut Frame, area: ratatui::layout::Rect) {
    let line = Line::from(Span::styled(
        " \u{2190}\u{2192} focus \u{2502} \u{2191}\u{2193} navigate \u{2502} click select \u{2502} PgUp/PgDn scroll \u{2502} Shift+click text \u{2502} q quit",
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(Paragraph::new(line), area);
}

/// Format the duration for a task based on its status and timestamps.
///
/// Returns an elapsed time string like `"3m12s"` or `"1h5m"`, or an empty
/// string when no timing information is available.
fn format_duration(task: &crate::task::Task) -> String {
    let seconds = match task.status {
        TaskStatus::Running => {
            let Some(started) = task.started_at else {
                return String::new();
            };
            let elapsed = Utc::now() - started;
            elapsed.num_seconds().max(0)
        }
        TaskStatus::Complete => {
            let (Some(started), Some(completed)) = (task.started_at, task.completed_at) else {
                return String::new();
            };
            let elapsed = completed - started;
            elapsed.num_seconds().max(0)
        }
        _ => return String::new(),
    };

    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{hours}h{minutes}m")
    } else {
        format!("{minutes}m{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::log_line::{LogLine, TodoItem, TodoStatus};
    use crate::task::{Task, TaskStatus};
    use chrono::{Duration, Utc};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn test_format_duration_running_task() {
        let started = Utc::now() - Duration::seconds(125);
        let task = Task {
            status: TaskStatus::Running,
            started_at: Some(started),
            ..Task::test_default()
        };
        let d = format_duration(&task);
        assert!(d.contains('m'), "expected minutes in duration: {d}");
        assert!(d.contains('s'), "expected seconds in duration: {d}");
    }

    #[test]
    fn test_format_duration_complete_task() {
        let started = Utc::now() - Duration::seconds(3700);
        let completed = Utc::now();
        let task = Task {
            status: TaskStatus::Complete,
            started_at: Some(started),
            completed_at: Some(completed),
            ..Task::test_default()
        };
        let d = format_duration(&task);
        assert!(d.contains('h'), "expected hours in duration: {d}");
    }

    #[test]
    fn test_format_duration_queued_task() {
        let task = Task {
            status: TaskStatus::Queued,
            ..Task::test_default()
        };
        assert_eq!(format_duration(&task), "");
    }

    #[test]
    fn test_format_duration_no_timestamps() {
        let task = Task {
            status: TaskStatus::Running,
            started_at: None,
            ..Task::test_default()
        };
        assert_eq!(format_duration(&task), "");
    }

    #[test]
    fn test_render_empty_state() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = TuiApp::new(4);

        terminal
            .draw(|frame| {
                render(&mut app, frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let header_line: String = (0..80)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();
        assert!(header_line.contains("TSK Server"));
        assert!(header_line.contains("Workers: 0/4"));
    }

    #[test]
    fn test_render_with_tasks() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = TuiApp::new(2);
        app.workers_active = 1;

        let started = Utc::now() - Duration::seconds(60);
        app.tasks = vec![
            Task {
                id: "t1".to_string(),
                name: "running-task".to_string(),
                task_type: "feat".to_string(),
                project: "myproject".to_string(),
                status: TaskStatus::Running,
                started_at: Some(started),
                branch_name: "tsk/feat/running-task/t1".to_string(),
                ..Task::test_default()
            },
            Task {
                id: "t2".to_string(),
                name: "done-task".to_string(),
                task_type: "fix".to_string(),
                project: "myproject".to_string(),
                status: TaskStatus::Complete,
                started_at: Some(started),
                completed_at: Some(Utc::now()),
                branch_name: "tsk/fix/done-task/t2".to_string(),
                ..Task::test_default()
            },
        ];

        terminal
            .draw(|frame| {
                render(&mut app, frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();

        // Check header shows active workers
        let header_line: String = (0..100)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();
        assert!(header_line.contains("Workers: 1/2"));

        // Check footer has key hints
        let footer_y = 29;
        let footer_line: String = (0..100)
            .map(|x| buffer[(x, footer_y)].symbol().to_string())
            .collect();
        assert!(footer_line.contains("quit"));
    }

    #[test]
    fn test_render_waiting_task() {
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = TuiApp::new(1);

        app.tasks = vec![Task {
            id: "child".to_string(),
            name: "waiting-child".to_string(),
            status: TaskStatus::Queued,
            parent_ids: vec!["parent-id".to_string()],
            branch_name: "tsk/feat/waiting-child/child".to_string(),
            ..Task::test_default()
        }];

        terminal
            .draw(|frame| {
                render(&mut app, frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        // Search for WAITING text in the rendered output
        let mut found_waiting = false;
        for y in 0..20 {
            let line: String = (0..50)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            if line.contains("WAITING") {
                found_waiting = true;
                break;
            }
        }
        assert!(found_waiting, "expected WAITING status in task list");
    }

    #[test]
    fn test_render_log_content() {
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = TuiApp::new(1);

        app.tasks = vec![Task {
            id: "t1".to_string(),
            name: "my-task".to_string(),
            branch_name: "tsk/feat/my-task/t1".to_string(),
            ..Task::test_default()
        }];
        app.log_content = vec![
            LogLine::message(vec![], None, "Log line one".into()),
            LogLine::message(vec![], None, "Log line two".into()),
        ];

        terminal
            .draw(|frame| {
                render(&mut app, frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();

        // Check the log panel title contains the task name
        let panel_start = app.task_panel_width as usize;
        let mut found_title = false;
        for y in 0..20 {
            let line: String = (panel_start..100)
                .map(|x| buffer[(x as u16, y)].symbol().to_string())
                .collect();
            if line.contains("my-task") {
                found_title = true;
                break;
            }
        }
        assert!(found_title, "expected task name in log panel title");

        // Check log content is rendered
        let mut found_log = false;
        for y in 0..20 {
            let line: String = (panel_start..100)
                .map(|x| buffer[(x as u16, y)].symbol().to_string())
                .collect();
            if line.contains("Log line one") {
                found_log = true;
                break;
            }
        }
        assert!(found_log, "expected log content to be rendered");
    }

    #[test]
    fn test_render_styled_log_lines() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = TuiApp::new(1);

        app.tasks = vec![Task {
            id: "t1".to_string(),
            name: "styled-task".to_string(),
            branch_name: "tsk/feat/styled-task/t1".to_string(),
            ..Task::test_default()
        }];
        app.log_content = vec![
            // Message with tags and tool
            LogLine::message(
                vec!["opus-4".into()],
                Some("Bash".into()),
                "Running: cargo test".into(),
            ),
            // Error message
            LogLine::error(vec![], Some("Bash".into()), "Tests failed".into()),
            // Todo items
            LogLine::todo(
                vec![],
                vec![
                    TodoItem {
                        content: "Done".into(),
                        status: TodoStatus::Completed,
                        active_form: None,
                        priority: None,
                    },
                    TodoItem {
                        content: "Working".into(),
                        status: TodoStatus::InProgress,
                        active_form: Some("Working on it".into()),
                        priority: None,
                    },
                ],
            ),
            // Summary
            LogLine::summary(true, "All done".into(), Some(0.15), Some(45000), Some(12)),
        ];

        terminal
            .draw(|frame| {
                render(&mut app, frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let panel_start = app.task_panel_width as usize;

        // Verify styled content renders
        let mut found_bash = false;
        let mut found_done = false;
        let mut found_success = false;
        for y in 0..30 {
            let line: String = (panel_start..120)
                .map(|x| buffer[(x as u16, y)].symbol().to_string())
                .collect();
            if line.contains("Bash") && line.contains("cargo test") {
                found_bash = true;
            }
            if line.contains("[x]") && line.contains("Done") {
                found_done = true;
            }
            if line.contains("SUCCESS") && line.contains("All done") {
                found_success = true;
            }
        }
        assert!(found_bash, "expected Bash tool line in rendered output");
        assert!(found_done, "expected completed todo in rendered output");
        assert!(found_success, "expected summary line in rendered output");

        // Verify render computed the accurate wrapped line count
        assert!(
            app.log_wrapped_line_count > 0,
            "expected log_wrapped_line_count to be set after render"
        );
    }
}
