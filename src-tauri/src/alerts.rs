use std::{
    collections::HashMap,
    process::Command,
    sync::Mutex,
    time::{Duration, Instant},
};

use tauri::AppHandle;

/// Rate-limited macOS notifications so alerts never spam.
/// Uses osascript because notification-center APIs require a signed app
/// bundle — this works from a bare dev binary too.
pub struct Alerter {
    last_fired: Mutex<HashMap<&'static str, Instant>>,
}

impl Alerter {
    pub fn new() -> Self {
        Self { last_fired: Mutex::new(HashMap::new()) }
    }

    pub fn fire(&self, _app: &AppHandle, key: &'static str, min_gap: Duration, title: &str, body: &str) {
        {
            let mut map = self.last_fired.lock().unwrap();
            if map.get(key).is_some_and(|t| t.elapsed() < min_gap) {
                return;
            }
            map.insert(key, Instant::now());
        }
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('"', "'"),
            title.replace('"', "'")
        );
        let _ = Command::new("osascript").arg("-e").arg(script).spawn();
    }
}
