use std::{fs, path::Path, process::Command};

use serde::Serialize;

use crate::{config::Config, history::Db, scanner};

#[derive(Serialize)]
pub struct CleanOutcome {
    pub moved: Vec<String>,
    pub errors: Vec<String>,
    pub total_bytes: u64,
}

/// Move paths to the macOS Trash — never a permanent delete.
pub fn clean_paths(cfg: &Config, db: &Db, paths: &[String]) -> CleanOutcome {
    let mut outcome = CleanOutcome { moved: Vec::new(), errors: Vec::new(), total_bytes: 0 };
    for p in paths {
        if cfg.never_touch.iter().any(|nt| p.starts_with(nt)) {
            outcome.errors.push(format!("{p}: on the never-touch list"));
            continue;
        }
        let path = Path::new(p);
        if !path.exists() {
            outcome.errors.push(format!("{p}: no longer exists"));
            continue;
        }
        let size = if path.is_dir() { scanner::dir_size(path) } else { path.metadata().map(|m| m.len()).unwrap_or(0) };
        match trash::delete(path) {
            Ok(()) => {
                db.log_cleaned(p, size);
                outcome.total_bytes += size;
                outcome.moved.push(p.clone());
            }
            Err(e) => outcome.errors.push(format!("{p}: {e}")),
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::Mutex;

    fn test_db() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE cleaned (ts INTEGER NOT NULL, path TEXT NOT NULL, size_bytes INTEGER NOT NULL);",
        )
        .unwrap();
        Db(Mutex::new(conn))
    }

    fn scratch_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("devspace-cleanup-test-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn never_touch_paths_are_skipped_and_not_deleted() {
        let dir = scratch_dir("never-touch");
        let precious = dir.join("precious.txt");
        fs::write(&precious, b"do not delete me").unwrap();
        let cfg = Config { never_touch: vec![dir.to_string_lossy().into_owned()], ..Default::default() };
        let db = test_db();

        let outcome = clean_paths(&cfg, &db, &[precious.to_string_lossy().into_owned()]);

        assert!(outcome.moved.is_empty());
        assert_eq!(outcome.errors.len(), 1);
        assert!(outcome.errors[0].contains("never-touch list"));
        assert!(precious.exists(), "never-touch path must not be trashed");
    }

    #[test]
    fn missing_path_reports_an_error_not_a_panic() {
        let cfg = Config::default();
        let db = test_db();
        let outcome = clean_paths(&cfg, &db, &["/nonexistent/devspace-test-path-xyz".into()]);
        assert!(outcome.moved.is_empty());
        assert_eq!(outcome.errors.len(), 1);
        assert!(outcome.errors[0].contains("no longer exists"));
    }

    #[test]
    fn successfully_trashed_paths_are_logged_and_totaled() {
        let dir = scratch_dir("trash-me");
        let junk = dir.join("junk.txt");
        fs::write(&junk, vec![0u8; 1024]).unwrap();
        let cfg = Config::default();
        let db = test_db();

        let outcome = clean_paths(&cfg, &db, &[junk.to_string_lossy().into_owned()]);

        assert_eq!(outcome.errors.len(), 0);
        assert_eq!(outcome.moved.len(), 1);
        assert_eq!(outcome.total_bytes, 1024);
        assert!(!junk.exists(), "trashed file should no longer be at its original path");
        assert_eq!(db.recently_cleaned().len(), 1, "trash success must be logged to history");
    }
}

#[derive(Serialize)]
pub struct HibernateOutcome {
    pub cleaned_bytes: u64,
    pub cleaned_paths: Vec<String>,
    pub marker_path: String,
    pub errors: Vec<String>,
}

/// Trash all regenerable-low content in the project and drop a marker file
/// recording what was removed and how to rebuild it.
pub fn hibernate_project(
    cfg: &Config,
    db: &Db,
    project_root: &str,
    regen_low_paths: &[String],
    rebuild_cmd: Option<&str>,
) -> Result<HibernateOutcome, String> {
    let root = Path::new(project_root);
    if !root.is_dir() {
        return Err(format!("{project_root} is not a directory"));
    }
    let in_project: Vec<String> = regen_low_paths
        .iter()
        .filter(|p| p.starts_with(project_root))
        .cloned()
        .collect();
    let outcome = clean_paths(cfg, db, &in_project);

    let marker = root.join(".devspace-hibernated.json");
    let record = serde_json::json!({
        "hibernated_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "removed": outcome.moved,
        "reclaimed_bytes": outcome.total_bytes,
        "rebuild_cmd": rebuild_cmd,
    });
    fs::write(&marker, serde_json::to_string_pretty(&record).unwrap())
        .map_err(|e| e.to_string())?;

    Ok(HibernateOutcome {
        cleaned_bytes: outcome.total_bytes,
        cleaned_paths: outcome.moved,
        marker_path: marker.to_string_lossy().into_owned(),
        errors: outcome.errors,
    })
}

#[derive(Serialize)]
pub struct RestoreOutcome {
    pub success: bool,
    pub output: String,
    /// Set instead of running anything when sandboxed — shelling out to run
    /// the rebuild command is forbidden under the App Sandbox. The frontend
    /// shows this so the user can run it themselves.
    pub manual_command: Option<String>,
}

/// Run the recorded rebuild command in the project and remove the marker.
/// Under the App Sandbox (`sandboxed: true`), shelling out to run it is
/// forbidden — the recorded command is returned instead of executed, and the
/// marker is left in place until the user restores it manually and re-scans.
pub fn restore_project(project_root: &str, sandboxed: bool) -> Result<RestoreOutcome, String> {
    let root = Path::new(project_root);
    let marker = root.join(".devspace-hibernated.json");
    let record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&marker).map_err(|e| format!("no hibernation marker: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let cmd = record["rebuild_cmd"]
        .as_str()
        .ok_or("no rebuild command recorded — restore manually")?
        .to_string();

    if sandboxed {
        return Ok(RestoreOutcome {
            success: false,
            output: String::new(),
            manual_command: Some(cmd),
        });
    }

    let out = Command::new("/bin/zsh")
        .arg("-lc")
        .arg(&cmd)
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        let _ = fs::remove_file(&marker);
    }
    Ok(RestoreOutcome {
        success: out.status.success(),
        output: text.chars().rev().take(4000).collect::<String>().chars().rev().collect(),
        manual_command: None,
    })
}
