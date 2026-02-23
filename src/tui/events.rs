/// Structured events emitted by the task scheduler.
///
/// When a `ServerEventSender` is provided, the scheduler sends events through
/// the channel instead of printing directly. When no sender is available,
/// the scheduler falls back to stdout/stderr output.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    /// A task has been scheduled for execution
    TaskScheduled { task_id: String, task_name: String },
    /// A task completed successfully
    TaskCompleted { task_id: String, task_name: String },
    /// A task failed with an error
    TaskFailed {
        task_id: String,
        task_name: String,
        error: String,
    },
    /// Informational status message
    StatusMessage(String),
    /// Warning or error message
    WarningMessage(String),
}

/// Sender half of the server event channel
pub type ServerEventSender = tokio::sync::mpsc::UnboundedSender<ServerEvent>;

/// Route an event through the TUI channel when available, otherwise print directly.
pub fn emit_or_print(sender: &Option<ServerEventSender>, event: ServerEvent) {
    if let Some(tx) = sender {
        let _ = tx.send(event);
    } else {
        match event {
            ServerEvent::TaskScheduled {
                task_name, task_id, ..
            } => {
                println!("Scheduling {task_name} ({task_id})");
            }
            ServerEvent::TaskCompleted {
                task_name, task_id, ..
            } => {
                println!("Task completed: {task_name} ({task_id})");
            }
            ServerEvent::TaskFailed {
                task_name,
                task_id,
                error,
            } => {
                eprintln!("Task failed: {task_name} ({task_id}) - {error}");
            }
            ServerEvent::StatusMessage(msg) => {
                println!("{msg}");
            }
            ServerEvent::WarningMessage(msg) => {
                eprintln!("{msg}");
            }
        }
    }
}
