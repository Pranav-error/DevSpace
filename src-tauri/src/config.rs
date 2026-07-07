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
