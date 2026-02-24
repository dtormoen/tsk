use crate::context::TaskStorage;
use crate::context::docker_client::DockerClient;
use crate::task::{Task, TaskStatus};
use crate::tui::app::TuiApp;
use crate::tui::events::ServerEvent;
use crate::tui::input::{TuiAction, handle_event};
use crate::tui::ui::render;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::collections::{HashMap, HashSet};
use std::io::Stdout;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{Duration, Instant, interval};

/// Guard that restores terminal state on drop, ensuring cleanup even on panic.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = self.terminal.show_cursor();
    }
}

/// Run the TUI event loop.
///
/// Sets up the terminal, then loops polling for crossterm input events,
/// server events from the scheduler, and periodic timers for refreshing
/// the task list and log content. Returns when the user presses `q` or
/// the server event channel closes.
///
/// `shutdown_notify` is signalled when the TUI wants the server to shut down.
pub async fn run_tui(
    mut event_receiver: UnboundedReceiver<ServerEvent>,
    storage: Arc<TaskStorage>,
    docker_client: Arc<dyn DockerClient>,
    data_dir: PathBuf,
    workers_total: usize,
    shutdown_notify: Arc<tokio::sync::Notify>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    let mut guard = TerminalGuard { terminal };
    let mut app = TuiApp::new(workers_total);

    // Load initial task list and sort for display
    refresh_task_list(&mut app, &storage).await;
    app.load_logs_for_selected_task(&data_dir);

    // Tick interval for periodic refreshes (task list + log content)
    let mut tick = interval(Duration::from_secs(1));
    tick.tick().await; // consume the immediate first tick

    // Crossterm event polling in a dedicated thread
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        loop {
            // Poll with 50ms timeout so the thread checks periodically
            if crossterm::event::poll(std::time::Duration::from_millis(50)).unwrap_or(false)
                && let Ok(event) = crossterm::event::read()
                && input_tx.send(event).is_err()
            {
                break;
            }
        }
    });

    let mut last_log_refresh = Instant::now();

    loop {
        // Draw the UI
        guard.terminal.draw(|frame| {
            render(&mut app, frame);
        })?;

        if app.should_quit {
            shutdown_notify.notify_one();
            break;
        }

        tokio::select! {
            // Crossterm input events
            Some(event) = input_rx.recv() => {
                if let Some(action) = handle_event(&mut app, &event, &data_dir) {
                    match action {
                        TuiAction::CancelTask { task_id, was_running, is_interactive } => {
                            match storage.mark_cancelled(&task_id).await {
                                Ok(_) => {
                                    app.server_messages.push((chrono::Local::now(), format!("Cancelled {task_id}")));
                                    if was_running {
                                        let container_name = if is_interactive {
                                            format!("tsk-interactive-{task_id}")
                                        } else {
                                            format!("tsk-{task_id}")
                                        };
                                        if let Err(e) = docker_client.kill_container(&container_name).await {
                                            app.server_messages.push((chrono::Local::now(), format!("Warning: could not kill container: {e}")));
                                        }
                                    }
                                }
                                Err(e) => {
                                    app.server_messages.push((chrono::Local::now(), format!("Failed to cancel {task_id}: {e}")));
                                }
                            }
                            refresh_task_list(&mut app, &storage).await;
                        }
                        TuiAction::DeleteTask { task_id, task_dir } => {
                            match storage.delete_task(&task_id).await {
                                Ok(_) => {
                                    if let Some(dir) = task_dir
                                        && crate::file_system::exists(&dir).await.unwrap_or(false)
                                        && let Err(e) = crate::file_system::remove_dir(&dir).await
                                    {
                                        app.server_messages.push((chrono::Local::now(), format!("Failed to delete task directory: {e}")));
                                    }
                                    app.server_messages.push((chrono::Local::now(), format!("Deleted {task_id}")));
                                }
                                Err(e) => {
                                    app.server_messages.push((chrono::Local::now(), format!("Failed to delete {task_id}: {e}")));
                                }
                            }
                            refresh_task_list(&mut app, &storage).await;
                        }
                    }
                }
            }
            // Server events from the scheduler
            event = event_receiver.recv() => {
                match event {
                    Some(server_event) => {
                        process_server_event(&mut app, &server_event, &storage).await;
                    }
                    None => {
                        // Channel closed — server has shut down
                        break;
                    }
                }
            }
            // Periodic tick for refreshing task list and logs
            _ = tick.tick() => {
                refresh_task_list(&mut app, &storage).await;
                // Also update worker counts from task data
                app.workers_active = app.tasks.iter()
                    .filter(|t| t.status == crate::task::TaskStatus::Running)
                    .count();
            }
        }

        // Refresh log content more frequently (every 500ms) for live tailing
        if last_log_refresh.elapsed() >= Duration::from_millis(500) {
            app.refresh_logs(&data_dir);
            last_log_refresh = Instant::now();
        }
    }

    // Guard's Drop handles terminal cleanup
    Ok(())
}

/// Reload tasks from storage, sort for display, and update the TUI.
async fn refresh_task_list(app: &mut TuiApp, storage: &TaskStorage) {
    if let Ok(tasks) = storage.list_tasks().await {
        let mut tasks: Vec<_> = tasks.into_iter().rev().collect();
        sort_tasks_for_display(&mut tasks);
        app.update_tasks(tasks);
    }
}

/// Process a server event, updating app state accordingly.
async fn process_server_event(app: &mut TuiApp, event: &ServerEvent, storage: &TaskStorage) {
    match event {
        ServerEvent::TaskScheduled {
            task_id: _,
            task_name,
        } => {
            app.server_messages
                .push((chrono::Local::now(), format!("Scheduling {task_name}")));
        }
        ServerEvent::TaskCompleted {
            task_id: _,
            task_name,
        } => {
            app.server_messages
                .push((chrono::Local::now(), format!("Task completed: {task_name}")));
        }
        ServerEvent::TaskFailed {
            task_id: _,
            task_name,
            error,
        } => {
            app.server_messages.push((
                chrono::Local::now(),
                format!("Task failed: {task_name} - {error}"),
            ));
        }
        ServerEvent::StatusMessage(msg) => {
            app.server_messages
                .push((chrono::Local::now(), msg.clone()));
        }
        ServerEvent::WarningMessage(msg) => {
            app.server_messages
                .push((chrono::Local::now(), format!("⚠ {msg}")));
        }
    }

    // Keep server messages bounded
    if app.server_messages.len() > 100 {
        app.server_messages.drain(..app.server_messages.len() - 100);
    }

    // Refresh task list after any server event
    refresh_task_list(app, storage).await;
}

/// Sort tasks for TUI display: non-terminal tasks first, then terminal tasks,
/// with children placed directly after their parent within each group.
///
/// Within each group, tasks without a parent-child relationship preserve their
/// reverse-chronological order (newest first).
pub(crate) fn sort_tasks_for_display(tasks: &mut Vec<Task>) {
    let is_terminal = |t: &Task| {
        matches!(
            t.status,
            TaskStatus::Complete | TaskStatus::Failed | TaskStatus::Cancelled
        )
    };

    // Split into non-terminal and terminal, preserving reverse-chrono order
    let (non_terminal, terminal): (Vec<_>, Vec<_>) = tasks.drain(..).partition(|t| !is_terminal(t));

    let non_terminal = arrange_with_parents(non_terminal);
    let terminal = arrange_with_parents(terminal);

    tasks.extend(non_terminal);
    tasks.extend(terminal);
}

/// Arrange tasks so children appear directly after their parent via DFS traversal.
///
/// Builds a parent→children map, identifies root tasks (no parent in this group),
/// then performs a depth-first pre-order traversal. Input order (reverse-chrono)
/// is preserved among roots and among siblings of the same parent.
fn arrange_with_parents(group: Vec<Task>) -> Vec<Task> {
    let group_ids: HashSet<String> = group.iter().map(|t| t.id.clone()).collect();

    // Map from parent ID → child indices (preserves input order = reverse-chrono)
    let mut children_map: HashMap<String, Vec<usize>> = HashMap::new();
    let mut root_indices: Vec<usize> = Vec::new();

    for (i, task) in group.iter().enumerate() {
        if let Some(parent_id) = task.parent_ids.first()
            && group_ids.contains(parent_id)
        {
            children_map.entry(parent_id.clone()).or_default().push(i);
        } else {
            root_indices.push(i);
        }
    }

    let mut slots: Vec<Option<Task>> = group.into_iter().map(Some).collect();
    let mut result: Vec<Task> = Vec::with_capacity(slots.len());

    fn dfs(
        idx: usize,
        slots: &mut [Option<Task>],
        children_map: &HashMap<String, Vec<usize>>,
        result: &mut Vec<Task>,
    ) {
        if let Some(task) = slots[idx].take() {
            if let Some(children) = children_map.get(&task.id) {
                let child_indices: Vec<usize> = children.clone();
                result.push(task);
                for child_idx in child_indices {
                    dfs(child_idx, slots, children_map, result);
                }
            } else {
                result.push(task);
            }
        }
    }

    for root_idx in root_indices {
        dfs(root_idx, &mut slots, &children_map, &mut result);
    }

    // Append orphans whose parent was in a different group
    for slot in &mut slots {
        if let Some(task) = slot.take() {
            result.push(task);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, status: TaskStatus, parent_ids: Vec<&str>) -> Task {
        Task {
            id: id.to_string(),
            status,
            parent_ids: parent_ids.into_iter().map(String::from).collect(),
            ..Task::test_default()
        }
    }

    fn ids(tasks: &[Task]) -> Vec<&str> {
        tasks.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn cancelled_tasks_are_terminal() {
        let mut tasks = vec![
            task("running", TaskStatus::Running, vec![]),
            task("cancelled", TaskStatus::Cancelled, vec![]),
            task("queued", TaskStatus::Queued, vec![]),
            task("complete", TaskStatus::Complete, vec![]),
        ];
        sort_tasks_for_display(&mut tasks);
        assert_eq!(
            ids(&tasks),
            vec!["running", "queued", "cancelled", "complete"]
        );
    }

    #[test]
    fn sort_is_idempotent() {
        let mut tasks = vec![
            task("running", TaskStatus::Running, vec![]),
            task("complete1", TaskStatus::Complete, vec![]),
            task("queued", TaskStatus::Queued, vec![]),
            task("failed", TaskStatus::Failed, vec![]),
            task("cancelled", TaskStatus::Cancelled, vec![]),
        ];
        sort_tasks_for_display(&mut tasks);
        let first_pass: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();

        sort_tasks_for_display(&mut tasks);
        let second_pass: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();

        assert_eq!(first_pass, second_pass);
    }

    #[test]
    fn parent_child_grouping_with_mixed_statuses() {
        let mut tasks = vec![
            task("parent-run", TaskStatus::Running, vec![]),
            task("child-run", TaskStatus::Running, vec!["parent-run"]),
            task("parent-done", TaskStatus::Complete, vec![]),
            task("child-done", TaskStatus::Complete, vec!["parent-done"]),
        ];
        sort_tasks_for_display(&mut tasks);
        assert_eq!(
            ids(&tasks),
            vec!["parent-run", "child-run", "parent-done", "child-done"]
        );
    }

    #[test]
    fn cancelled_parent_with_cancelled_children() {
        let mut tasks = vec![
            task("parent", TaskStatus::Cancelled, vec![]),
            task("child", TaskStatus::Cancelled, vec!["parent"]),
            task("running", TaskStatus::Running, vec![]),
        ];
        sort_tasks_for_display(&mut tasks);
        assert_eq!(ids(&tasks), vec!["running", "parent", "child"]);
    }
}
