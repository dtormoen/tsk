#[cfg(target_os = "linux")]
use notify_rust::Hint;
use notify_rust::{Notification, Timeout};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Trait for sending desktop notifications
pub trait NotificationClient: Send + Sync {
    /// Notify when a task completes
    fn notify_task_complete(&self, task_name: &str, success: bool, message: Option<&str>);

    /// Enable or disable sound notifications
    fn set_sound_enabled(&self, enabled: bool);
}

/// Desktop notification client using notify-rust
pub struct DesktopNotificationClient {
    timeout_seconds: u32,
    sound_enabled: AtomicBool,
}

impl DesktopNotificationClient {
    /// Create a new desktop notification client
    pub fn new(timeout_seconds: u32, sound_enabled: bool) -> Self {
        Self {
            timeout_seconds,
            sound_enabled: AtomicBool::new(sound_enabled),
        }
    }

    /// Play sound on macOS using afplay (fire-and-forget)
    #[cfg(target_os = "macos")]
    fn play_macos_sound(&self, success: bool) {
        let sound = if success {
            "/System/Library/Sounds/Glass.aiff"
        } else {
            "/System/Library/Sounds/Basso.aiff"
        };
        let _ = std::process::Command::new("afplay").arg(sound).spawn();
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

        // Add sound hint for Linux (freedesktop-compliant desktops)
        #[cfg(target_os = "linux")]
        if self.sound_enabled.load(Ordering::Relaxed) {
            let sound_name = if success {
                "message-new-instant"
            } else {
                "dialog-warning"
            };
            notification.hint(Hint::SoundName(sound_name.into()));
        }

        if let Err(e) = notification.show() {
            // Fall back to terminal output
            eprintln!("TSK: {} - {}", summary, body.replace('\n', " "));
            eprintln!("(Desktop notification failed: {e})");
        }

        // Play sound separately on macOS (hints don't work there)
        #[cfg(target_os = "macos")]
        if self.sound_enabled.load(Ordering::Relaxed) {
            self.play_macos_sound(success);
        }
    }

    fn set_sound_enabled(&self, enabled: bool) {
        self.sound_enabled.store(enabled, Ordering::Relaxed);
    }
}

/// No-op notification client for testing
pub struct NoOpNotificationClient;

impl NotificationClient for NoOpNotificationClient {
    fn notify_task_complete(&self, _task_name: &str, _success: bool, _message: Option<&str>) {
        // No-op
    }

    fn set_sound_enabled(&self, _enabled: bool) {
        // No-op
    }
}

/// Create a notification client based on the environment
pub fn create_notification_client() -> Arc<dyn NotificationClient> {
    // Check if we're in a test environment or CI
    if std::env::var("TSK_NO_NOTIFICATIONS").is_ok() || cfg!(test) {
        Arc::new(NoOpNotificationClient)
    } else {
        Arc::new(DesktopNotificationClient::new(5, false))
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
    }

    #[test]
    fn test_create_notification_client_in_tests() {
        let client = create_notification_client();
        // Should return NoOpNotificationClient in tests
        client.notify_task_complete("test", true, None);
    }

    #[test]
    fn test_set_sound_enabled() {
        let client = create_notification_client();
        // Verify set_sound_enabled can be called without error
        client.set_sound_enabled(true);
        client.set_sound_enabled(false);
        client.notify_task_complete("test", true, None);
    }

    #[test]
    fn test_desktop_notification_client_sound_toggle() {
        let client = DesktopNotificationClient::new(5, false);
        // Default should be false
        assert!(!client.sound_enabled.load(Ordering::Relaxed));

        client.set_sound_enabled(true);
        assert!(client.sound_enabled.load(Ordering::Relaxed));

        client.set_sound_enabled(false);
        assert!(!client.sound_enabled.load(Ordering::Relaxed));
    }
}
