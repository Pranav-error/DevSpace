use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

/// Rate-limited macOS notifications so alerts never spam.
/// Uses the tauri notification plugin (UserNotifications under the hood, via
/// the notify-rust crate) — no shell-out, App Sandbox-safe. This replaced an
/// earlier osascript-based implementation kept only for unsigned dev binaries;
/// the plugin works the same in dev and in a signed/notarized or sandboxed
/// build.
pub struct Alerter {
    last_fired: Mutex<HashMap<&'static str, Instant>>,
}

impl Alerter {
    pub fn new() -> Self {
        Self { last_fired: Mutex::new(HashMap::new()) }
    }

    pub fn fire(&self, app: &AppHandle, key: &'static str, min_gap: Duration, title: &str, body: &str) {
        {
            let mut map = self.last_fired.lock().unwrap();
            if map.get(key).is_some_and(|t| t.elapsed() < min_gap) {
                return;
            }
            map.insert(key, Instant::now());
        }
        let _ = app.notification().builder().title(title).body(body).show();
    }
}
