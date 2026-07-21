//! Security-scoped bookmarks for the sandboxed (App Store) edition.
//!
//! Under the App Sandbox a folder the user grants via the open panel is only
//! reachable again after a relaunch if we persist a *security-scoped bookmark*
//! for it and re-acquire access before touching the folder. This module stores
//! those bookmarks (base64 in a small JSON file) and hands out RAII guards that
//! start/stop access around a scan.
//!
//! It degrades gracefully: if a bookmark can't be created or resolved (e.g.
//! running unsandboxed in `cargo run`, or macOS refuses), the caller simply
//! falls back to the plain path, which still works outside the sandbox.

// `Access` is a public RAII guard type returned by `access`; callers hold it
// anonymously (`let _guard = ...`), so the name import reads as unused.
#[cfg(target_os = "macos")]
#[allow(unused_imports)]
pub use imp::{access, remove_bookmark, save_bookmark, Access};

#[cfg(not(target_os = "macos"))]
#[allow(unused_imports)]
pub use stub::{access, remove_bookmark, save_bookmark, Access};

#[cfg(target_os = "macos")]
mod imp {
    use std::{collections::HashMap, path::PathBuf};

    use base64::{engine::general_purpose::STANDARD, Engine};
    use objc2::rc::Retained;
    use objc2_foundation::{
        NSData, NSString, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSURL,
    };

    use crate::config::devspace_dir;

    fn store_path() -> PathBuf {
        devspace_dir().join("bookmarks.json")
    }

    fn load_store() -> HashMap<String, String> {
        std::fs::read_to_string(store_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save_store(m: &HashMap<String, String>) {
        if let Ok(s) = serde_json::to_string_pretty(m) {
            let _ = std::fs::write(store_path(), s);
        }
    }

    /// Create + persist a security-scoped bookmark for a granted folder.
    /// A no-op (leaving no entry) if the bookmark can't be created.
    pub fn save_bookmark(path: &str) {
        let Some(bytes) = create(path) else { return };
        let mut m = load_store();
        m.insert(path.to_string(), STANDARD.encode(&bytes));
        save_store(&m);
    }

    /// Forget a folder's bookmark when the user removes it.
    pub fn remove_bookmark(path: &str) {
        let mut m = load_store();
        if m.remove(path).is_some() {
            save_store(&m);
        }
    }

    fn create(path: &str) -> Option<Vec<u8>> {
        let ns_path = NSString::from_str(path);
        let url = NSURL::fileURLWithPath_isDirectory(&ns_path, true);
        let data = url
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::WithSecurityScope,
                None,
                None,
            )
            .ok()?;
        Some(data.to_vec())
    }

    /// Resolve stored bookmarks for `roots` and begin accessing them. The
    /// returned guards must stay alive for the whole scan; dropping them stops
    /// access. Roots without a bookmark are simply skipped (the caller still
    /// has the plain path).
    pub fn access(roots: &[String]) -> Vec<Access> {
        let store = load_store();
        let mut guards = Vec::new();
        for root in roots {
            let Some(b64) = store.get(root) else { continue };
            let Ok(bytes) = STANDARD.decode(b64) else { continue };
            let data = NSData::with_bytes(&bytes);
            let resolved = unsafe {
                NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                    &data,
                    NSURLBookmarkResolutionOptions::WithSecurityScope,
                    None,
                    std::ptr::null_mut(), // staleness unused
                )
            };
            let Ok(url) = resolved else { continue };
            let ok = unsafe { url.startAccessingSecurityScopedResource() };
            if ok {
                guards.push(Access { url });
            }
        }
        guards
    }

    /// RAII: stops accessing the security-scoped resource on drop.
    pub struct Access {
        url: Retained<NSURL>,
    }

    impl Drop for Access {
        fn drop(&mut self) {
            unsafe { self.url.stopAccessingSecurityScopedResource() };
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod stub {
    pub struct Access;
    pub fn save_bookmark(_path: &str) {}
    pub fn remove_bookmark(_path: &str) {}
    pub fn access(_roots: &[String]) -> Vec<Access> {
        Vec::new()
    }
}
