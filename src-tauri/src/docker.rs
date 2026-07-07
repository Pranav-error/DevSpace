use std::process::Command;

use serde::Serialize;

#[derive(Serialize)]
pub struct DockerImage {
    pub id: String,
    pub repo_tag: String,
    pub size_bytes: u64,
    /// "in-use" (a container references it), "dangling" (<none> tag), or "unused".
    pub status: String,
}

#[derive(Serialize)]
pub struct DockerContainer {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub size_bytes: u64,
}

#[derive(Serialize)]
pub struct DockerVolume {
    pub name: String,
    pub size_bytes: u64,
    pub in_use: bool,
}

#[derive(Serialize)]
pub struct DockerInfo {
    pub available: bool,
    pub error: Option<String>,
    pub images: Vec<DockerImage>,
    pub containers: Vec<DockerContainer>,
    pub volumes: Vec<DockerVolume>,
    pub build_cache_bytes: u64,
}

fn docker(args: &[&str]) -> Result<String, String> {
    let out = Command::new("docker")
        .args(args)
        .output()
        .map_err(|e| format!("docker not available: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Docker prints sizes like "1.23GB", "456MB", "789kB".
fn parse_size(s: &str) -> u64 {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let num: f64 = s[..split].parse().unwrap_or(0.0);
    let mult = match s[split..].to_ascii_uppercase().as_str() {
        "TB" => 1e12,
        "GB" => 1e9,
        "MB" => 1e6,
        "KB" => 1e3,
        _ => 1.0,
    };
    (num * mult) as u64
}

pub fn info() -> DockerInfo {
    let mut result = DockerInfo {
        available: false,
        error: None,
        images: Vec::new(),
        containers: Vec::new(),
        volumes: Vec::new(),
        build_cache_bytes: 0,
    };

    // Containers first — they tell us which images are in use.
    let containers = match docker(&["ps", "-a", "--format", "{{json .}}"]) {
        Ok(out) => out,
        Err(e) => {
            result.error = Some(e);
            return result;
        }
    };
    result.available = true;

    let mut used_images: Vec<String> = Vec::new();
    for line in containers.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let image = v["Image"].as_str().unwrap_or("").to_string();
        used_images.push(image.clone());
        result.containers.push(DockerContainer {
            id: v["ID"].as_str().unwrap_or("").into(),
            name: v["Names"].as_str().unwrap_or("").into(),
            image,
            state: v["State"].as_str().unwrap_or("").into(),
            size_bytes: parse_size(v["Size"].as_str().unwrap_or("0B").split(' ').next().unwrap_or("0B")),
        });
    }

    if let Ok(out) = docker(&["images", "--format", "{{json .}}"]) {
        for line in out.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let repo = v["Repository"].as_str().unwrap_or("<none>");
            let tag = v["Tag"].as_str().unwrap_or("<none>");
            let repo_tag = format!("{repo}:{tag}");
            let status = if repo == "<none>" || tag == "<none>" {
                "dangling"
            } else if used_images.iter().any(|u| u == &repo_tag || u == repo) {
                "in-use"
            } else {
                "unused"
            };
            result.images.push(DockerImage {
                id: v["ID"].as_str().unwrap_or("").into(),
                repo_tag,
                size_bytes: parse_size(v["Size"].as_str().unwrap_or("0B")),
                status: status.into(),
            });
        }
    }

    if let Ok(out) = docker(&["system", "df", "--format", "{{json .}}"]) {
        for line in out.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            if v["Type"].as_str() == Some("Build Cache") {
                result.build_cache_bytes = parse_size(v["Size"].as_str().unwrap_or("0B"));
            }
        }
    }

    if let Ok(out) = docker(&["volume", "ls", "--format", "{{json .}}"]) {
        // `docker system df -v` has per-volume sizes but is expensive; volume
        // list is enough for v1 of this tab (size unknown → 0).
        for line in out.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            result.volumes.push(DockerVolume {
                name: v["Name"].as_str().unwrap_or("").into(),
                size_bytes: 0,
                in_use: false,
            });
        }
    }

    result.images.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    result
}

pub fn remove(kind: &str, id: &str) -> Result<String, String> {
    match kind {
        "image" => docker(&["rmi", id]),
        "container" => docker(&["rm", id]),
        "volume" => docker(&["volume", "rm", id]),
        "build-cache" => docker(&["builder", "prune", "-f"]),
        _ => Err(format!("unknown kind {kind}")),
    }
}
