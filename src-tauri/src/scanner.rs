use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Serialize;
use walkdir::WalkDir;

use crate::config::{home_dir, Config};

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Bucket {
    RegenLow,
    RegenMedium,
    Precious,
    /// Not a dev artifact, just big (≥500MB) — shown so nothing eating the
    /// disk stays invisible.
    LargeFile,
}

#[derive(Clone, Serialize)]
pub struct Finding {
    pub path: String,
    /// What this is, e.g. "node_modules", "conda env", "checkpoint".
    pub kind: String,
    pub bucket: Bucket,
    pub size_bytes: u64,
    /// Project root this belongs to, if under a scan root.
    pub project: Option<String>,
    /// Medium-risk envs with no lockfile/spec in the project get flagged.
    pub rebuild_risk: bool,
}

#[derive(Clone, Serialize)]
pub struct ProjectInfo {
    pub root: String,
    pub name: String,
    /// Unix seconds of the newest non-regenerable file.
    pub last_touched: u64,
    pub days_idle: u64,
    /// Bytes of regenerable-low content (what hibernation would reclaim).
    pub regen_low_bytes: u64,
    pub hibernated: bool,
    pub rebuild_cmd: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ScanResult {
    pub findings: Vec<Finding>,
    pub projects: Vec<ProjectInfo>,
    pub scanned_at: u64,
}

/// Dir names that are regenerable-low: record, size, never descend.
const REGEN_LOW_DIRS: &[&str] = &[
    "node_modules",
    ".next",
    "dist",
    "build",
    "__pycache__",
    ".pytest_cache",
    "DerivedData",
    ".turbo",
    ".parcel-cache",
    ".nuxt",
    "target", // sized only when a Cargo.toml sibling exists (checked below)
];

/// Dir names that are regenerable-medium (rebuildable from a spec file).
const REGEN_MEDIUM_DIRS: &[&str] = &[".venv", "venv", ".tox"];

const PRECIOUS_EXTS: &[&str] = &[
    "pt", "pth", "ckpt", "safetensors", "gguf", "h5", "onnx",
];
/// .bin is precious only above this size (tiny .bin files are everywhere).
const PRECIOUS_BIN_MIN: u64 = 50 * 1024 * 1024;
/// Ignore precious files smaller than this to keep the list useful.
const PRECIOUS_MIN: u64 = 10 * 1024 * 1024;
/// Anything at least this big is worth showing even if it's not dev-related.
const LARGE_FILE_MIN: u64 = 500 * 1024 * 1024;

pub fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn mtime_secs(md: &fs::Metadata) -> u64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn build_ignore(cfg: &Config) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for pat in &cfg.ignore_patterns {
        if let Ok(g) = Glob::new(pat) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

fn detect_rebuild_cmd(project: &Path) -> Option<String> {
    if project.join("package.json").exists() {
        return Some("npm install".into());
    }
    if project.join("requirements.txt").exists() {
        return Some("pip install -r requirements.txt".into());
    }
    if project.join("environment.yml").exists() {
        return Some("conda env create -f environment.yml".into());
    }
    if project.join("pyproject.toml").exists() {
        return Some("pip install -e .".into());
    }
    if project.join("Cargo.toml").exists() {
        return Some("cargo build".into());
    }
    None
}

/// Does the project have a spec that makes its env rebuildable?
fn has_env_spec(project: &Path) -> bool {
    ["requirements.txt", "environment.yml", "pyproject.toml", "Pipfile", "setup.py"]
        .iter()
        .any(|f| project.join(f).exists())
}

pub fn scan<F: FnMut(&str, usize)>(cfg: &Config, mut progress: F) -> ScanResult {
    let ignore = build_ignore(cfg);
    let mut findings: Vec<Finding> = Vec::new();
    let mut projects: Vec<ProjectInfo> = Vec::new();
    let mut visited = 0usize;

    for root in &cfg.scan_roots {
        let root_path = PathBuf::from(root);
        if !root_path.is_dir() {
            continue;
        }
        // Each direct child dir of a scan root is treated as a project.
        let children: Vec<PathBuf> = fs::read_dir(&root_path)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
                    .collect()
            })
            .unwrap_or_default();

        for project in children {
            let project_str = project.to_string_lossy().into_owned();
            if cfg.never_touch.iter().any(|p| project_str.starts_with(p)) {
                continue;
            }
            progress(&project_str, visited);

            let mut last_touched = 0u64;
            let mut regen_low_bytes = 0u64;

            let walker = WalkDir::new(&project).follow_links(false).into_iter();
            let ignore_ref = &ignore;
            let mut it = walker.filter_entry(move |e| {
                let name = e.file_name().to_string_lossy();
                if e.file_type().is_dir() && (name == ".git" || ignore_ref.is_match(e.path())) {
                    return false;
                }
                true
            });

            // Unreadable entries (permissions, broken links) are skipped, not fatal.
            while let Some(next) = it.next() {
                let Ok(entry) = next else { continue };
                visited += 1;
                if visited % 2000 == 0 {
                    progress(&entry.path().to_string_lossy(), visited);
                }
                let path = entry.path();
                if ignore.is_match(path) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();

                if entry.file_type().is_dir() {
                    let is_target = name == "target";
                    let regen_low = REGEN_LOW_DIRS.contains(&name.as_str())
                        && (!is_target || path.parent().is_some_and(|p| p.join("Cargo.toml").exists()));
                    let regen_med = REGEN_MEDIUM_DIRS.contains(&name.as_str())
                        && path.join("pyvenv.cfg").exists()
                        || (REGEN_MEDIUM_DIRS.contains(&name.as_str()) && name != "venv");

                    if regen_low || regen_med {
                        let size = dir_size(path);
                        if regen_low {
                            regen_low_bytes += size;
                        }
                        if size > 1024 * 1024 {
                            findings.push(Finding {
                                path: path.to_string_lossy().into_owned(),
                                kind: name.clone(),
                                bucket: if regen_low { Bucket::RegenLow } else { Bucket::RegenMedium },
                                size_bytes: size,
                                project: Some(project_str.clone()),
                                rebuild_risk: !regen_low && !has_env_spec(&project),
                            });
                        }
                        it.skip_current_dir();
                        continue;
                    }
                } else if entry.file_type().is_file() {
                    if let Ok(md) = entry.metadata() {
                        last_touched = last_touched.max(mtime_secs(&md));
                        let ext = path
                            .extension()
                            .map(|e| e.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        let size = md.len();
                        let precious = (PRECIOUS_EXTS.contains(&ext.as_str()) && size >= PRECIOUS_MIN)
                            || (ext == "bin" && size >= PRECIOUS_BIN_MIN)
                            || name == ".env";
                        if precious {
                            findings.push(Finding {
                                path: path.to_string_lossy().into_owned(),
                                kind: if name == ".env" { "env file".into() } else { "checkpoint".into() },
                                bucket: Bucket::Precious,
                                size_bytes: size,
                                project: Some(project_str.clone()),
                                rebuild_risk: false,
                            });
                        } else if size >= LARGE_FILE_MIN {
                            findings.push(Finding {
                                path: path.to_string_lossy().into_owned(),
                                kind: "large file".into(),
                                bucket: Bucket::LargeFile,
                                size_bytes: size,
                                project: Some(project_str.clone()),
                                rebuild_risk: false,
                            });
                        }
                    }
                }
            }

            let days_idle = if last_touched == 0 {
                0
            } else {
                now_secs().saturating_sub(last_touched) / 86_400
            };
            projects.push(ProjectInfo {
                name: project
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| project_str.clone()),
                hibernated: project.join(".devspace-hibernated.json").exists(),
                rebuild_cmd: detect_rebuild_cmd(&project),
                root: project_str,
                last_touched,
                days_idle,
                regen_low_bytes,
            });
        }
    }

    // The blocks below reach into fixed home-dir locations, which the App
    // Sandbox forbids — the sandboxed edition scans only user-granted roots.
    if cfg.sandboxed {
        findings.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        projects.sort_by(|a, b| b.days_idle.cmp(&a.days_idle));
        return ScanResult {
            findings,
            projects,
            scanned_at: now_secs(),
        };
    }

    // Conda environments outside scan roots.
    for conda_root in ["miniconda3/envs", "anaconda3/envs", ".conda/envs", "miniforge3/envs"] {
        let dir = home_dir().join(conda_root);
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for env in rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()) {
            progress(&env.to_string_lossy(), visited);
            let size = dir_size(&env);
            if size > 10 * 1024 * 1024 {
                findings.push(Finding {
                    path: env.to_string_lossy().into_owned(),
                    kind: "conda env".into(),
                    bucket: Bucket::RegenMedium,
                    size_bytes: size,
                    project: None,
                    rebuild_risk: true, // can't know which project owns it
                });
            }
        }
    }

    // Well-known hidden caches that quietly eat tens of GB.
    // Medium = re-downloadable model stores; Low = plain caches.
    let known_caches: &[(&str, &str, Bucket)] = &[
        (".ollama/models", "ollama models (re-pull with `ollama pull`)", Bucket::RegenMedium),
        (".cache/huggingface", "huggingface cache", Bucket::RegenMedium),
        (".npm", "npm cache", Bucket::RegenLow),
        (".cache/pip", "pip cache", Bucket::RegenLow),
        (".cargo/registry", "cargo registry cache", Bucket::RegenLow),
        (".gradle", "gradle cache", Bucket::RegenLow),
        (".pub-cache", "pub (dart/flutter) cache", Bucket::RegenLow),
        (".dartServer", "dart analysis cache", Bucket::RegenLow),
        (".gemini", "gemini CLI data", Bucket::RegenMedium),
        (".vscode/extensions", "VSCode extensions (reinstallable)", Bucket::RegenMedium),
        ("Library/Caches", "app caches (apps rebuild these)", Bucket::RegenMedium),
        ("Library/Developer/Xcode/DerivedData", "Xcode DerivedData", Bucket::RegenLow),
        ("Library/Developer/CoreSimulator", "iOS simulators", Bucket::RegenMedium),
        ("Library/pnpm", "pnpm store", Bucket::RegenLow),
    ];
    for (rel, kind, bucket) in known_caches {
        let dir = home_dir().join(rel);
        if !dir.is_dir() {
            continue;
        }
        progress(&dir.to_string_lossy(), visited);
        let size = dir_size(&dir);
        if size > 100 * 1024 * 1024 {
            findings.push(Finding {
                path: dir.to_string_lossy().into_owned(),
                kind: (*kind).into(),
                bucket: *bucket,
                size_bytes: size,
                project: None,
                rebuild_risk: false,
            });
        }
    }

    findings.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    projects.sort_by(|a, b| b.days_idle.cmp(&a.days_idle));

    ScanResult {
        findings,
        projects,
        scanned_at: now_secs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn write(p: &Path, bytes: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; bytes]).unwrap();
    }

    fn fixture(label: &str) -> (PathBuf, ScanResult) {
        let root = std::env::temp_dir().join(format!("devspace-scan-test-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("myproj");
        write(&proj.join("node_modules/pkg/index.js"), 2 * 1024 * 1024);
        write(&proj.join(".venv/pyvenv.cfg"), 10);
        write(&proj.join(".venv/lib/dep.bin"), 2 * 1024 * 1024);
        write(&proj.join("model.safetensors"), 11 * 1024 * 1024);
        write(&proj.join(".env"), 100);
        write(&proj.join("package.json"), 10);
        write(&proj.join("src/main.js"), 500);
        // Inside .git must be pruned entirely.
        write(&proj.join(".git/objects/huge.pack"), 12 * 1024 * 1024);
        let cfg = Config {
            scan_roots: vec![root.to_string_lossy().into_owned()],
            ignore_patterns: vec![],
            ..Default::default()
        };
        let res = scan(&cfg, |_, _| {});
        (root, res)
    }

    #[test]
    fn classifier_buckets_are_correct() {
        let (root, res) = fixture("buckets");
        let in_fixture: Vec<&Finding> = res
            .findings
            .iter()
            .filter(|f| f.path.starts_with(root.to_string_lossy().as_ref()))
            .collect();

        let find = |k: &str| {
            in_fixture
                .iter()
                .find(|f| f.kind == k)
                .unwrap_or_else(|| panic!("missing finding kind {k}"))
        };
        assert!(matches!(find("node_modules").bucket, Bucket::RegenLow));
        assert!(matches!(find(".venv").bucket, Bucket::RegenMedium));
        assert!(matches!(find("checkpoint").bucket, Bucket::Precious));
        assert!(matches!(find("env file").bucket, Bucket::Precious));
        // .git contents must never be reported.
        assert!(!in_fixture.iter().any(|f| f.path.contains("/.git/")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn project_metadata_detected() {
        let (root, res) = fixture("meta");
        let p = res
            .projects
            .iter()
            .find(|p| p.name == "myproj")
            .expect("project missing");
        assert_eq!(p.rebuild_cmd.as_deref(), Some("npm install"));
        // node_modules counts toward hibernation-reclaimable bytes.
        assert!(p.regen_low_bytes >= 2 * 1024 * 1024);
        assert!(!p.hibernated);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn venv_without_spec_is_flagged_risky() {
        let (root, res) = fixture("risk");
        let venv = res
            .findings
            .iter()
            .find(|f| f.kind == ".venv" && f.path.starts_with(root.to_string_lossy().as_ref()))
            .unwrap();
        // package.json is not a python env spec → risky.
        assert!(venv.rebuild_risk);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dir_size_sums_files() {
        let root = std::env::temp_dir().join(format!("devspace-size-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        write(&root.join("a/b.bin"), 1000);
        write(&root.join("c.bin"), 500);
        assert_eq!(dir_size(&root), 1500);
        let _ = fs::remove_dir_all(&root);
    }
}
