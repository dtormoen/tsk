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
