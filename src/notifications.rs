use notify_rust::{Notification, Timeout};
use std::sync::Arc;

/// Trait for sending desktop notifications
pub trait NotificationClient: Send + Sync {
    /// Notify when a task completes
    fn notify_task_complete(&self, task_name: &str, success: bool, message: Option<&str>);

    /// Notify when all tasks are complete
    fn notify_all_tasks_complete(&self, total: usize, succeeded: usize, failed: usize);
}

/// Desktop notification client using notify-rust
pub struct DesktopNotificationClient {
    timeout_seconds: u32,
}

impl DesktopNotificationClient {
    /// Create a new desktop notification client
    pub fn new(timeout_seconds: u32) -> Self {
        Self { timeout_seconds }
    }
}

impl NotificationClient for DesktopNotificationClient {
    fn notify_task_complete(&self, task_name: &str, success: bool, message: Option<&str>) {
        let summary = if success {
            "Task Completed"
        } else {
            "Task Failed"
        };
        let body = format!(
            "Task '{}' has {}\n{}",
            task_name,
            if success {
                "completed successfully"
            } else {
                "failed"
            },
            message.unwrap_or("")
        );

        let mut notification = Notification::new();
        notification.summary(summary);
        notification.body(&body);
        notification.timeout(Timeout::Milliseconds(self.timeout_seconds * 1000));

        if let Err(e) = notification.show() {
            // Fall back to terminal output
            eprintln!("TSK: {} - {}", summary, body.replace('\n', " "));
            eprintln!("(Desktop notification failed: {e})");
        }
    }

    fn notify_all_tasks_complete(&self, total: usize, succeeded: usize, failed: usize) {
        let summary = if failed == 0 {
            "All Tasks Completed"
        } else {
            "Tasks Completed with Failures"
        };

        let body = format!("Total: {total}\nSucceeded: {succeeded}\nFailed: {failed}");

        let mut notification = Notification::new();
        notification.summary(summary);
        notification.body(&body);
        notification.timeout(Timeout::Milliseconds(self.timeout_seconds * 1000));

        if let Err(e) = notification.show() {
            // Fall back to terminal output
            eprintln!("TSK: {} - {}", summary, body.replace('\n', " "));
            eprintln!("(Desktop notification failed: {e})");
        }
    }
}

/// No-op notification client for testing
pub struct NoOpNotificationClient;

impl NotificationClient for NoOpNotificationClient {
    fn notify_task_complete(&self, _task_name: &str, _success: bool, _message: Option<&str>) {
        // No-op
    }

    fn notify_all_tasks_complete(&self, _total: usize, _succeeded: usize, _failed: usize) {
        // No-op
    }
}

/// Create a notification client based on the environment
pub fn create_notification_client() -> Arc<dyn NotificationClient> {
    // Check if we're in a test environment or CI
    if std::env::var("TSK_NO_NOTIFICATIONS").is_ok() || cfg!(test) {
        Arc::new(NoOpNotificationClient)
    } else {
        Arc::new(DesktopNotificationClient::new(5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_notification_client() {
        let client = NoOpNotificationClient;
        // Should not panic
        client.notify_task_complete("test", true, None);
        client.notify_task_complete("test", false, Some("error"));
        client.notify_all_tasks_complete(5, 3, 2);
    }

    #[test]
    fn test_create_notification_client_in_tests() {
        let client = create_notification_client();
        // Should return NoOpNotificationClient in tests
        client.notify_task_complete("test", true, None);
    }
}
