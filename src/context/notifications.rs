#[cfg(all(target_os = "linux", not(test)))]
use notify_rust::{Hint, Notification, Timeout};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Desktop notification client for task completion events
pub struct NotificationClient {
    #[cfg(all(target_os = "linux", not(test)))]
    timeout_seconds: u32,
    sound_enabled: AtomicBool,
}

impl NotificationClient {
    /// Create a new notification client
    #[cfg(all(target_os = "linux", not(test)))]
    pub fn new(timeout_seconds: u32, sound_enabled: bool) -> Self {
        Self {
            timeout_seconds,
            sound_enabled: AtomicBool::new(sound_enabled),
        }
    }

    /// Create a new notification client
    #[cfg(any(not(target_os = "linux"), test))]
    pub fn new(_timeout_seconds: u32, sound_enabled: bool) -> Self {
        Self {
            sound_enabled: AtomicBool::new(sound_enabled),
        }
    }

    /// Show notification on macOS using osascript (fire-and-forget)
    #[cfg(all(target_os = "macos", not(test)))]
    fn show_macos_notification(&self, title: &str, message: &str, sound_enabled: bool) {
        let sound_part = if sound_enabled {
            " sound name \"Glass\""
        } else {
            ""
        };
        let script = format!(
            "display notification \"{}\" with title \"{}\"{}",
            message.replace('\"', "\\\"").replace('\n', " "),
            title.replace('\"', "\\\""),
            sound_part
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }

    /// Notify when a task completes
    #[cfg(not(test))]
    pub fn notify_task_complete(&self, task_name: &str, success: bool, message: Option<&str>) {
        let summary = if success {
            "Task Completed"
        } else {
            "Task Failed"
        };
        let body = format!(
            "Task '{}' has {}{}",
            task_name,
            if success {
                "completed successfully"
            } else {
                "failed"
            },
            message.map(|m| format!(": {}", m)).unwrap_or_default()
        );

        // On macOS, use osascript which is reliable and non-blocking
        #[cfg(target_os = "macos")]
        {
            self.show_macos_notification(
                summary,
                &body,
                self.sound_enabled.load(Ordering::Relaxed),
            );
        }

        // On Linux, use notify-rust with sound hints
        #[cfg(target_os = "linux")]
        {
            let timeout_seconds = self.timeout_seconds;
            let sound_enabled = self.sound_enabled.load(Ordering::Relaxed);
            let sound_name = if success {
                "message-new-instant"
            } else {
                "dialog-warning"
            };

            let mut notification = Notification::new();
            notification.summary(summary);
            notification.body(&body);
            notification.timeout(Timeout::Milliseconds(timeout_seconds * 1000));

            if sound_enabled {
                notification.hint(Hint::SoundName(sound_name.into()));
            }

            if let Err(e) = notification.show() {
                eprintln!("TSK: {} - {}", summary, body.replace('\n', " "));
                eprintln!("(Desktop notification failed: {e})");
            }
        }
    }

    /// Notify when a task completes (no-op in test builds)
    #[cfg(test)]
    pub fn notify_task_complete(&self, _task_name: &str, _success: bool, _message: Option<&str>) {}

    /// Enable or disable sound notifications
    pub fn set_sound_enabled(&self, enabled: bool) {
        self.sound_enabled.store(enabled, Ordering::Relaxed);
    }
}

/// Create a notification client based on the environment
pub fn create_notification_client() -> Arc<NotificationClient> {
    Arc::new(NotificationClient::new(5, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_use_notification_client() {
        let client = create_notification_client();
        client.set_sound_enabled(true);
        client.set_sound_enabled(false);
        client.notify_task_complete("test", true, None);
    }

    #[test]
    fn test_notification_client_sound_toggle() {
        let client = NotificationClient::new(5, false);
        // Default should be false
        assert!(!client.sound_enabled.load(Ordering::Relaxed));

        client.set_sound_enabled(true);
        assert!(client.sound_enabled.load(Ordering::Relaxed));

        client.set_sound_enabled(false);
        assert!(!client.sound_enabled.load(Ordering::Relaxed));
    }
}
