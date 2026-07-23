use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directories the disk scanner walks.
    pub scan_roots: Vec<String>,
    /// Glob patterns the scanner skips entirely.
    pub ignore_patterns: Vec<String>,
    /// Absolute paths the app must never offer to clean.
    pub never_touch: Vec<String>,
    /// Memory poll interval for the menu bar readout.
    pub poll_interval_secs: u64,
    /// Fire a memory alert at or above this used percentage.
    pub mem_warn_pct: f64,
    /// Fire a disk alert below this many free GB (also the forecast target).
    pub disk_warn_gb: f64,
    /// Suggest hibernating projects untouched for this many days.
    pub hibernation_age_days: u64,
    /// Show live text next to the tray icon.
    pub tray_live_text: bool,
    /// App Store edition: scan only user-granted folders (security-scoped
    /// bookmarks) and skip the home-dir conda/cache probing the sandbox forbids.
    pub sandboxed: bool,
}

impl Default for Config {
    fn default() -> Self {
        let home = home_dir();
        let mut scan_roots = Vec::new();
        for candidate in [
            "Documents/GitHub",
            "code",
            "Developer",
            "Projects",
            "dev",
        ] {
            let p = home.join(candidate);
            if p.is_dir() {
                scan_roots.push(p.to_string_lossy().into_owned());
            }
        }
        Self {
            scan_roots,
            ignore_patterns: vec!["**/Library/**".into(), "**/.Trash/**".into()],
            never_touch: Vec::new(),
            poll_interval_secs: 3,
            mem_warn_pct: 85.0,
            disk_warn_gb: 10.0,
            hibernation_age_days: 60,
            tray_live_text: true,
            // App Store edition default: sandboxed, folders granted by the user.
            sandboxed: true,
        }
    }
}

pub fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set"))
}

pub fn devspace_dir() -> PathBuf {
    let dir = home_dir().join(".devspace");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn config_path() -> PathBuf {
    devspace_dir().join("config.json")
}

pub fn load() -> Config {
    match fs::read_to_string(config_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => {
            let cfg = Config::default();
            let _ = save(&cfg);
            cfg
        }
    }
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(config_path(), text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn default_is_sandboxed() {
        // The App Store edition must default to sandboxed — Phase 1/2 gating
        // throughout the app (scanner, Docker/ML UI, restore_project, quit_app)
        // all key off this.
        assert!(Config::default().sandboxed);
    }

    #[test]
    fn default_has_expected_thresholds() {
        let cfg = Config::default();
        assert_eq!(cfg.mem_warn_pct, 85.0);
        assert_eq!(cfg.disk_warn_gb, 10.0);
        assert_eq!(cfg.hibernation_age_days, 60);
        assert!(cfg.tray_live_text);
        assert!(cfg.never_touch.is_empty());
        assert!(cfg
            .ignore_patterns
            .iter()
            .any(|p| p == "**/Library/**"));
    }

    #[test]
    fn deserializing_json_missing_sandboxed_field_defaults_to_true() {
        // Real-world regression case hit this session: a config.json written
        // before the `sandboxed` field existed. #[serde(default)] on the
        // struct must fill it in from Config::default() (true), not false or
        // a deserialize error, or every pre-existing install would silently
        // un-sandbox itself.
        let stale_json = r#"{
            "scan_roots": ["/Users/x/Documents/GitHub/a"],
            "ignore_patterns": ["**/Library/**"],
            "never_touch": [],
            "poll_interval_secs": 3,
            "mem_warn_pct": 70.0,
            "disk_warn_gb": 10.0,
            "hibernation_age_days": 60,
            "tray_live_text": true
        }"#;
        let cfg: Config = serde_json::from_str(stale_json).unwrap();
        assert!(cfg.sandboxed, "missing field must default to true, not false");
        assert_eq!(cfg.mem_warn_pct, 70.0, "present fields must be preserved, not overwritten by defaults");
        assert_eq!(cfg.scan_roots, vec!["/Users/x/Documents/GitHub/a"]);
    }

    #[test]
    fn round_trips_through_json() {
        let mut cfg = Config::default();
        cfg.scan_roots.push("/tmp/somewhere".into());
        cfg.mem_warn_pct = 72.5;
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.scan_roots, cfg.scan_roots);
        assert_eq!(back.mem_warn_pct, 72.5);
        assert_eq!(back.sandboxed, cfg.sandboxed);
    }
}
