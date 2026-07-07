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
}

/// Run the recorded rebuild command in the project and remove the marker.
pub fn restore_project(project_root: &str) -> Result<RestoreOutcome, String> {
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
    })
}
