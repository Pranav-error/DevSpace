use std::{fs, path::Path, process::Command};

use serde::Serialize;

use crate::config::devspace_dir;

/// Python helper used for FP16 conversion and quantization. Written to
/// ~/.devspace/helpers/convert.py and executed with the system python3.
const CONVERT_PY: &str = r#"
import json, os, shutil, sys

def fail(msg):
    print(json.dumps({"ok": False, "error": msg}))
    sys.exit(0)

def main():
    path, mode = sys.argv[1], sys.argv[2]
    before = os.path.getsize(path)
    backup = path + ".devspace-backup"
    try:
        import torch
    except ImportError:
        fail("PyTorch is not installed for python3. Run: pip3 install torch")

    is_safetensors = path.endswith(".safetensors")
    try:
        if is_safetensors:
            try:
                from safetensors.torch import load_file, save_file
            except ImportError:
                fail("safetensors is not installed. Run: pip3 install safetensors")
            tensors = load_file(path)
        else:
            tensors = torch.load(path, map_location="cpu", weights_only=False)

        def convert(obj):
            if torch.is_tensor(obj):
                if mode == "fp16":
                    return obj.half() if obj.is_floating_point() else obj
                if mode == "int8":
                    if obj.is_floating_point():
                        q = torch.quantize_per_tensor(obj.float(), scale=obj.abs().max().item() / 127 or 1e-8, zero_point=0, dtype=torch.qint8)
                        return q
                    return obj
            if isinstance(obj, dict):
                return {k: convert(v) for k, v in obj.items()}
            if isinstance(obj, (list, tuple)):
                t = [convert(v) for v in obj]
                return type(obj)(t) if isinstance(obj, tuple) else t
            return obj

        converted = convert(tensors)
        shutil.copy2(path, backup)
        if is_safetensors:
            if mode == "int8":
                fail("INT8 quantized tensors can't be stored in safetensors; use fp16 or a .pt file")
            save_file(converted, path)
        else:
            torch.save(converted, path)
        after = os.path.getsize(path)
        os.remove(backup)
        print(json.dumps({"ok": True, "before": before, "after": after}))
    except Exception as e:
        if os.path.exists(backup):
            shutil.move(backup, path)
        fail(str(e))

main()
"#;

#[derive(Serialize, Debug)]
pub struct ArchiveOutcome {
    pub ok: bool,
    pub message: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub new_path: Option<String>,
}

/// External volumes a checkpoint could be moved to.
pub fn list_volumes() -> Vec<String> {
    fs::read_dir("/Volumes")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir()
                        && !p
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy() == "Macintosh HD")
                })
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Move a file to an external volume and symlink it back so training
/// scripts keep working.
pub fn move_and_symlink(path: &str, volume: &str) -> Result<ArchiveOutcome, String> {
    let src = Path::new(path);
    if !src.is_file() {
        return Err(format!("{path} is not a file"));
    }
    if src.is_symlink() {
        return Err("already a symlink — probably archived before".into());
    }
    let size = src.metadata().map(|m| m.len()).unwrap_or(0);
    let dest_dir = Path::new(volume).join("DevSpaceArchive");
    fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    let dest = dest_dir.join(src.file_name().ok_or("bad filename")?);
    if dest.exists() {
        return Err(format!("{} already exists on the volume", dest.display()));
    }
    // fs::rename fails across filesystems; copy + verify size + remove.
    fs::copy(src, &dest).map_err(|e| format!("copy failed: {e}"))?;
    let copied = dest.metadata().map(|m| m.len()).unwrap_or(0);
    if copied != size {
        let _ = fs::remove_file(&dest);
        return Err("size mismatch after copy — original left untouched".into());
    }
    fs::remove_file(src).map_err(|e| e.to_string())?;
    std::os::unix::fs::symlink(&dest, src).map_err(|e| e.to_string())?;
    Ok(ArchiveOutcome {
        ok: true,
        message: format!("moved to {} and symlinked back", dest.display()),
        before_bytes: size,
        after_bytes: size,
        new_path: Some(dest.to_string_lossy().into_owned()),
    })
}

/// Convert a checkpoint in place (fp16 or int8) via the python helper.
/// Lossy and not bit-identical — the UI warns before calling this.
pub fn convert(path: &str, mode: &str) -> Result<ArchiveOutcome, String> {
    if !matches!(mode, "fp16" | "int8") {
        return Err(format!("unknown mode {mode}"));
    }
    let helper_dir = devspace_dir().join("helpers");
    fs::create_dir_all(&helper_dir).map_err(|e| e.to_string())?;
    let helper = helper_dir.join("convert.py");
    fs::write(&helper, CONVERT_PY).map_err(|e| e.to_string())?;

    // Prefer DevSpace's private venv (~/.devspace/venv) so torch doesn't
    // have to pollute the system python (Homebrew blocks that via PEP 668).
    let venv_python = devspace_dir().join("venv/bin/python");
    let python = if venv_python.exists() {
        venv_python
    } else {
        std::path::PathBuf::from("python3")
    };
    let out = Command::new(python)
        .arg(&helper)
        .arg(path)
        .arg(mode)
        .output()
        .map_err(|e| format!("python3 not available: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim().lines().last().unwrap_or("{}"))
        .map_err(|_| format!("helper produced no result: {stdout}"))?;
    if parsed["ok"].as_bool() == Some(true) {
        Ok(ArchiveOutcome {
            ok: true,
            message: format!("converted to {mode} in place"),
            before_bytes: parsed["before"].as_u64().unwrap_or(0),
            after_bytes: parsed["after"].as_u64().unwrap_or(0),
            new_path: None,
        })
    } else {
        Err(parsed["error"].as_str().unwrap_or("unknown error").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("devspace-archive-test-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn convert_rejects_unknown_mode_without_touching_python() {
        // Validated before the python helper is ever written/spawned — should
        // error immediately regardless of whether python/torch are installed.
        let err = convert("/tmp/whatever.pt", "int4").unwrap_err();
        assert!(err.contains("unknown mode"));
    }

    #[test]
    fn move_and_symlink_rejects_non_file_source() {
        let dir = scratch_dir("non-file");
        let err = move_and_symlink(&dir.to_string_lossy(), "/tmp").unwrap_err();
        assert!(err.contains("not a file"));
    }

    #[test]
    fn move_and_symlink_rejects_already_symlinked_source() {
        let dir = scratch_dir("already-symlink");
        let real = dir.join("real.pt");
        fs::write(&real, b"data").unwrap();
        let link = dir.join("checkpoint.pt");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let err = move_and_symlink(&link.to_string_lossy(), "/tmp").unwrap_err();
        assert!(err.contains("already a symlink"));
    }

    #[test]
    fn move_and_symlink_moves_file_and_leaves_a_working_symlink() {
        let source_dir = scratch_dir("move-source");
        let volume_dir = scratch_dir("move-volume");
        let checkpoint = source_dir.join("model.safetensors");
        let payload = vec![7u8; 4096];
        fs::write(&checkpoint, &payload).unwrap();

        let outcome = move_and_symlink(&checkpoint.to_string_lossy(), &volume_dir.to_string_lossy()).unwrap();

        assert!(outcome.ok);
        assert_eq!(outcome.before_bytes, 4096);
        assert_eq!(outcome.after_bytes, 4096);
        assert!(checkpoint.is_symlink(), "original path must become a symlink");
        assert_eq!(
            fs::read(&checkpoint).unwrap(),
            payload,
            "reading through the symlink must still return the original content"
        );
        let archived = volume_dir.join("DevSpaceArchive").join("model.safetensors");
        assert!(archived.is_file(), "the real file must now live under DevSpaceArchive on the volume");
    }

    #[test]
    fn move_and_symlink_refuses_to_overwrite_existing_dest() {
        let source_dir = scratch_dir("dup-source");
        let volume_dir = scratch_dir("dup-volume");
        let checkpoint = source_dir.join("model.pt");
        fs::write(&checkpoint, b"data").unwrap();
        let archive_subdir = volume_dir.join("DevSpaceArchive");
        fs::create_dir_all(&archive_subdir).unwrap();
        fs::write(archive_subdir.join("model.pt"), b"already here").unwrap();

        let err = move_and_symlink(&checkpoint.to_string_lossy(), &volume_dir.to_string_lossy()).unwrap_err();
        assert!(err.contains("already exists"));
        assert!(checkpoint.is_file(), "original must be untouched when the dest collides");
        assert!(!checkpoint.is_symlink());
    }
}
