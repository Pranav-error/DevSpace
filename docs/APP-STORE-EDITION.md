# DevSpace — Mac App Store Edition Plan

> Status: **planning** (2026-07-21). The direct-distribution app (notarized, on GitHub
> Releases + Homebrew) stays the primary, full-featured build. This document scopes a
> **separate, reduced SKU** — "DevSpace Lite" — that can pass App Review.

## TL;DR

The App Store edition is **not a config flag** on the current app. Three current pillars
are fundamentally incompatible with the App Sandbox and App Review:

1. **Private API** (`macOSPrivateApi` + `tauri-nspanel`) → automatic rejection.
2. **App Sandbox** confinement → breaks free filesystem scanning, Docker, and process control.
3. **Shelling out to external binaries** (`docker`, `/bin/zsh`, `osascript`, venv `python`) → forbidden in the sandbox.

The edition is therefore a **feature-reduced fork of the UI shell + scanner**, re-architected
around user-granted folder access. Everything precious about the full app (Docker cleanup,
process killing, ML quantization, whole-home scanning) is *dropped* in this SKU.

---

## What breaks, and why (grounded in current code)

| Feature | Code location | Why it fails App Store | Fate in Lite |
|---|---|---|---|
| Vibrancy popover / transparency | `tauri.conf.json` `macOSPrivateApi:true`, `windowEffects`; `Cargo.toml` `macos-private-api` | Tauri's private-API feature is an explicit App Store rejection | **Replace** with opaque window (`backgroundColor`, no vibrancy) |
| Float over other apps' fullscreen | `lib.rs:211-221` `to_panel()`, `set_level(25)`, `set_style_mask`, tauri-nspanel | NSPanel conversion via private-API-tainted crate | **Drop** floating-over-fullscreen; normal menu-bar popover |
| Quit / kill other apps | `lib.rs:294` osascript ⌘Q, `lib.rs:302` `kill` | Sandbox forbids controlling/terminating other processes | **Drop** the Quit/kill controls |
| Docker cleanup | `docker.rs:41` `Command::new("docker")` | Can't exec external binaries in sandbox | **Drop** the Docker tab |
| Trash cleanup via shell | `cleanup.rs:106` `Command::new("/bin/zsh")` | No shell-out; also needs FS access | **Rewrite** using the `trash` crate directly, scoped to granted folders |
| Native notifications | `alerts.rs:35` `osascript` | No `osascript`; needs entitlement | **Replace** with UserNotifications (tauri notification plugin) |
| ML checkpoint quantize/archive | `archive.rs:152` `Command::new(python)` (venv torch) | Can't run bundled/external python | **Drop** in v1 Lite (revisit as a pure-Rust candle path later) |
| Whole-home / conda / caches scan | `scanner.rs` walks `home_dir()`, conda roots, `~/.ollama`, `Library/Caches` | Sandbox blocks arbitrary home traversal | **Rewrite** around security-scoped bookmarks (user grants folders) |
| Top processes / per-app RAM | `sysinfo` process enumeration | Sandbox restricts other-process introspection; per-process detail unreliable | **Reduce** to total RAM + own usage; verify what survives on-device |

---

## Target architecture for "DevSpace Lite"

### 1. Window shell (public API only)
- `macOSPrivateApi: false`; remove `macos-private-api` from `Cargo.toml` features.
- Remove the `tauri-nspanel` dependency and all of `lib.rs`'s panel code.
- Keep the Accessory (`LSUIElement`) activation policy + tray icon — those are public.
- Popover = a normal borderless, opaque Tauri window shown under the tray rect, hidden on
  blur. **Accepted tradeoff:** it will *not* float over another app's fullscreen space.
  (If we later want that back within App Store rules, it's doable with public `NSWindow`
  `level`/`collectionBehavior` via `objc2` — but it's out of scope for Lite v1.)

### 2. Filesystem access = security-scoped bookmarks (the DaisyDisk model)
- First run: an onboarding pane asks the user to **drag in / pick folders** to monitor
  (e.g. `~/Developer`, `~/Documents/GitHub`). Each grant → a security-scoped bookmark.
- Persist bookmarks; on launch call `startAccessingSecurityScopedResource` before scanning,
  stop after. The scanner (`WalkDir`) only ever runs **inside granted roots** — no `home_dir()`
  traversal, no hardcoded conda/cache paths.
- Entitlements: `com.apple.security.app-sandbox`, `com.apple.security.files.user-selected.read-write`,
  and the bookmark entitlements (`...files.bookmarks.app-scope`).
- The 3-bucket classifier (regenerable-low / regenerable-medium / precious) is **reusable as-is**
  — it's pure logic over paths; only the *roots* change.

### 3. Cleanup
- Keep Trash-only safety rule. Use the `trash` crate directly (no `/bin/zsh`).
- Only offer deletion for items **within granted, user-selected folders**.

### 4. Notifications
- Swap `osascript` alerts for the tauri notification plugin (UserNotifications). Add the
  entitlement; request permission on first alert.

### 5. Signing / build / submission (mechanical)
- Cert: **Apple Distribution** (not Developer ID) + a **Mac App Store provisioning profile**
  tied to bundle id `com.saipranav.devspace` (or a distinct id like `com.saipranav.devspace-lite`
  to ship both editions side by side — **recommended** so the two don't collide).
- Build → sign with App Sandbox + hardened-runtime entitlements → package `.pkg` →
  upload via **Transporter** / `xcrun altool` → **App Review**.
- Screenshots, privacy nutrition label (we read disk usage locally, send nothing → "no data
  collected"), category (Developer Tools / Utilities).

---

## Phasing

- **Phase 0 — spike (½ day):** new branch `app-store-edition`. Flip off private API, delete
  nspanel, get the opaque popover launching from the tray. Confirms the shell survives without
  private API before investing in the scanner rewrite.
- **Phase 1 — bookmark scanner:** onboarding folder-grant flow + security-scoped bookmark
  persistence; scanner reads only granted roots; classifier reused.
- **Phase 2 — Trash cleanup + notifications** inside the sandbox.
- **Phase 3 — sandbox entitlements, Apple Distribution signing, first TestFlight/Review build.**

## Decisions (resolved 2026-07-21)
1. **Separate bundle id: `com.saipranav.devspace-lite`** — both editions can coexist on one Mac. ✅
2. **Drop floating-over-fullscreen in Lite v1** — the private-API path that enabled it is the main
   rejection cause; not worth the risk. ✅
3. **Free** — Lite is a strict subset of the free open-source app; the App Store listing is purely
   for discoverability + trust. ✅

## Why direct distribution stays primary
For a power-user dev tool, the notarized GitHub/Homebrew build is the *better* channel: no
sandbox (keeps Docker, process control, home-wide scan, ML quantization), no review latency,
no 15–30% cut. The App Store edition is about **discoverability + trust for less technical
users**, at the cost of most of the power features.
