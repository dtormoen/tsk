use crate::context::TaskStorage;
use crate::tui::app::TuiApp;
use crate::tui::events::ServerEvent;
use crate::tui::input::handle_event;
use crate::tui::ui::render;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
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

    // Load initial task list
    if let Ok(tasks) = storage.list_tasks().await {
        app.update_tasks(tasks);
        app.load_logs_for_selected_task(&data_dir);
    }

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
                handle_event(&mut app, &event, &data_dir);
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
                if let Ok(tasks) = storage.list_tasks().await {
                    app.update_tasks(tasks);
                }
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
    if let Ok(tasks) = storage.list_tasks().await {
        app.update_tasks(tasks);
    }
}
