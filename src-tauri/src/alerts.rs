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
        if !self.should_fire(key, min_gap) {
            return;
        }
        let _ = app.notification().builder().title(title).body(body).show();
    }

    /// The debounce decision, pulled out of `fire` so it's testable without a
    /// real Tauri `AppHandle` (which the notification call itself needs).
    /// Returns true (and records `now`) at most once per `min_gap` per key.
    fn should_fire(&self, key: &'static str, min_gap: Duration) -> bool {
        let mut map = self.last_fired.lock().unwrap();
        if map.get(key).is_some_and(|t| t.elapsed() < min_gap) {
            return false;
        }
        map.insert(key, Instant::now());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_fire_for_a_key_is_always_allowed() {
        let alerter = Alerter::new();
        assert!(alerter.should_fire("disk", Duration::from_secs(3600)));
    }

    #[test]
    fn repeat_within_min_gap_is_suppressed() {
        let alerter = Alerter::new();
        assert!(alerter.should_fire("disk", Duration::from_secs(3600)));
        // Immediately again, well inside the 1-hour gap — must be suppressed,
        // this is the whole point of the rate limit (never spam the user).
        assert!(!alerter.should_fire("disk", Duration::from_secs(3600)));
    }

    #[test]
    fn different_keys_are_independent() {
        let alerter = Alerter::new();
        assert!(alerter.should_fire("disk", Duration::from_secs(3600)));
        // A different alert key must not be blocked by "disk"'s cooldown.
        assert!(alerter.should_fire("memory", Duration::from_secs(3600)));
    }

    #[test]
    fn allowed_again_once_the_gap_has_elapsed() {
        let alerter = Alerter::new();
        assert!(alerter.should_fire("forecast", Duration::from_millis(20)));
        assert!(!alerter.should_fire("forecast", Duration::from_millis(20)));
        std::thread::sleep(Duration::from_millis(30));
        assert!(alerter.should_fire("forecast", Duration::from_millis(20)));
    }
}
