use std::sync::OnceLock;

static NOTIFICATIONS_AVAILABLE: OnceLock<bool> = OnceLock::new();

pub fn is_available() -> bool {
    *NOTIFICATIONS_AVAILABLE.get_or_init(|| {
        if cfg!(target_os = "macos") {
            true
        } else if cfg!(target_os = "linux") {
            std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
        } else {
            false
        }
    })
}

pub fn send_notification(title: &str, body: &str) -> anyhow::Result<()> {
    notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show()
        .map(|_| ())
        .map_err(|e| {
            log::warn!("Failed to send OS notification: {}", e);
            anyhow::anyhow!("notification error: {}", e)
        })
}
