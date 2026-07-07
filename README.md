# DevSpace

**A macOS menu bar app for developers whose Macs keep running out of RAM and disk.**

DevSpace lives in your menu bar with a live readout (`72% · 10G`). Click it and a native-feeling popover opens: see exactly which apps eat your RAM and quit them, scan your dev folders and clean gigabytes of regenerable junk to the Trash, manage ML checkpoints without ever risking them, surgically clean Docker, hibernate dead projects, and get told *in advance* when your disk will run out.

Built with **Tauri v2** (Rust + vanilla JS) — idles under 10 MB of RAM, no Electron.

---

## Why another cleaner?

ClearDisk, DevClean, and friends all do generic dev-cache cleanup. DevSpace's angle:

1. **ML-aware** — it knows `node_modules` is disposable but `.pt` / `.safetensors` / `.ckpt` checkpoints are precious. Precious files are *never* deletable in the UI — only archivable.
2. **Unified RAM + disk** — one tool, both problems, one menu bar icon.
3. **Surgical Docker cleanup** — dangling vs unused vs in-use images, per-item removal. No nuke-everything button.
4. **Checkpoint archiving** — move to an external drive with a symlink back (training scripts keep working), or compress in place with FP16/INT8 quantization.

## Features

### 🖥 Monitor (live, every 3s)
- Memory bar with **your** configurable warning threshold (color + visible warning line + macOS notification, rate-limited)
- **Processes grouped by app** — VSCode's 40 helper processes show as one row: `Visual Studio Code · 40 processes · 2.6 GB · 16%`
- **Click the Memory section** to expand from top 5 to the top 20 RAM consumers (scrollable, live-updating)
- **Hover an app → Quit button** — graceful ⌘Q with save prompts, never a force-kill. CLI/background groups (your terminal sessions) are protected and never get a Quit button
- Live tray text (`72% · 10G`), toggleable; Pause/Resume from the tray menu
- **Storage forecasting**: hourly disk samples in SQLite, recency-weighted linear trend, "you'll hit 10 GB free in ~18 days" with a proactive notification under 14 days

### 💾 Disk scanner (on demand)
Walks your configured roots and classifies everything:

| Bucket | Examples | UI treatment |
|---|---|---|
| 🟢 Safe to clean | `node_modules`, `dist`, `.next`, `__pycache__`, `target`, DerivedData, npm/pip/cargo/gradle/pnpm caches, Xcode data | checkbox, select-all |
| 🟡 Rebuildable | `.venv`, conda envs, **ollama models**, HuggingFace cache, VSCode extensions — flagged ⚠ when no lockfile exists to rebuild from | checkbox, warned |
| 🔵 Large files | anything ≥500 MB that isn't a dev artifact (forgotten videos, datasets, ISOs) | checkbox |
| 🟣 Precious | `.pt` `.pth` `.ckpt` `.safetensors` `.gguf` `.h5` `.onnx`, big `.bin`, `.env` files | **no checkbox — archive only** |

Also detects well-known hidden space hogs most tools miss: `~/.ollama/models`, `~/.cache/huggingface`, `~/.gradle`, `~/Library/Caches`, iOS simulators, and more.

### 🗑 Safe cleanup
- Everything goes to the **Trash** (`trash` crate) — never `rm -rf`, always recoverable
- Preview-then-confirm modal with exact paths, every time
- 24h "recently cleaned" undo list + cumulative "Total saved" counter (SQLite)
- User-defined **never-touch** paths are refused even if selected

### 🧠 ML tab
Every checkpoint on your machine in one place, **grouped by project** with per-project totals, biggest first. Deletion is impossible here by design. Per-file archive actions (always confirmed, size preview first):
- **Move to external volume + symlink back** — lossless; copy is size-verified before the original is touched; `torch.load(...)` keeps working through the symlink
- **FP16 in place** — ~50% smaller, ~zero accuracy loss for inference weights
- **INT8 in place** — ~75% smaller, clearly flagged lossy (blocked for `.safetensors`, which can't store quantized tensors)

Conversions run through DevSpace's private Python environment (`~/.devspace/venv`) with automatic backup-and-restore: a failed conversion leaves the original byte-identical.

### 🐳 Docker
- Images split into dangling / unused / **in-use (protected — no Remove button)**
- Per-image and per-container removal, build-cache prune. Nothing bulk, ever.

### 😴 Project hibernation
- Tracks last-touched per project; suggests hibernating ones idle 60+ days
- Hibernate = regenerable content to Trash + a `.devspace-hibernated.json` marker recording how to rebuild (`npm install`, `pip install -r requirements.txt`, …)
- One-click Restore runs the recorded rebuild command

### ✨ Native feel
- Non-activating `NSPanel` (via [tauri-nspanel](https://github.com/ahkohd/tauri-nspanel)) — floats over **fullscreen apps**, closes on click-outside/Esc, never steals your Space
- Real macOS vibrancy, spring physics everywhere (CSS `linear()` damped-spring curves), sliding segmented-control indicator, content-sized popover per tab

## Install / build

Prereqs: Rust (rustup), Node.js, Xcode Command Line Tools.

```bash
git clone https://github.com/Pranav-error/DevSpace.git
cd DevSpace
npm install
npm run tauri dev      # run in dev mode
npm run tauri build    # produce DevSpace.app + .dmg
```

Optional, for checkpoint FP16/INT8 conversion:

```bash
python3 -m venv ~/.devspace/venv
~/.devspace/venv/bin/pip install torch
```

Config lives at `~/.devspace/config.json` (scan roots, thresholds, ignore globs, never-touch paths, hibernation age) and is editable from the Settings tab. History (cleanup log, disk samples) lives in `~/.devspace/history.db`.

## Testing

```bash
cd src-tauri && cargo test
```

10 unit tests cover the risky logic: classifier buckets, `.git` pruning, rebuild-command detection, lockfile risk flags, directory sizing, forecast regression (declining/stable/insufficient data), cleanup logging, Docker size parsing, and process→app grouping. The FP16 converter and its crash-safety (corrupt input → original untouched) are verified end-to-end. The manual QA checklist is in [`docs/BETA-TESTING.md`](docs/BETA-TESTING.md).

## Architecture

Five parts, two clocks:

1. **Shell** — tray icon + NSPanel popover. Dumb, just renders.
2. **Memory engine** — continuous 3s `sysinfo` poll, read-only.
3. **Disk engine** — on-demand background scan (walkdir with pruning) feeding the classifier.
4. **Rules/config** — shared JSON both engines read.
5. **Alerts** — cross-cutting, rate-limited notifications (osascript, works unbundled).

The two engines run on different clocks (continuous vs on-demand) by design.

## Safety principles

These hold for every feature, forever:

- Destructive actions go to the **Trash**, never permanent delete
- Always **preview → confirm** with exact paths
- **Precious files can't be deleted** through the UI at all
- Quantization is never automatic and always labeled lossy
- The app can quit *your* apps only gracefully (⌘Q semantics) and refuses to touch CLI/terminal sessions

## Stack

- [Tauri v2](https://tauri.app) — Rust backend, macOS system webview
- [sysinfo](https://crates.io/crates/sysinfo) · [walkdir](https://crates.io/crates/walkdir) · [trash](https://crates.io/crates/trash) · [rusqlite](https://crates.io/crates/rusqlite) · [globset](https://crates.io/crates/globset) · [tauri-nspanel](https://github.com/ahkohd/tauri-nspanel)
- Vanilla HTML/CSS/JS frontend — no framework

## Roadmap

- [x] v1 — menu bar live readout
- [x] v1.5 — live tray text, template icon, pause/resume, standalone .app
- [x] v2 — disk scanner + 4-bucket classifier + hidden-cache detection
- [x] v3 — Trash-only cleanup, undo window, never-touch list
- [x] v4 — Docker surgical cleanup + ML checkpoint archiver (symlink / FP16 / INT8)
- [x] v5 — project hibernation + storage forecasting
- [x] Unit tests + beta test script
- [ ] Notarized releases (Developer ID) + Homebrew cask
- [ ] Pure-Rust checkpoint conversion via [candle](https://github.com/huggingface/candle) — drop the Python dependency
- [ ] **App Store edition** — sandboxed variant using user-granted folder access (DaisyDisk-style): scanner, cleanup, ML tab with candle conversion; no Docker/app-quit
- [ ] FSEvents-based hibernation auto-restore
- [ ] Per-project growth forecasting

**Distribution:** this full version ships as notarized direct downloads (GitHub Releases, later Homebrew). The Mac App Store sandbox can't accommodate everything here (Docker CLI, quitting apps, free-roaming scans), so the App Store edition above will be a reduced variant using security-scoped folder grants.

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) — especially the non-negotiable safety rules. Licensed under [MIT](LICENSE).

---

*Personal project by a CS student who got tired of deleting Docker to fit WhatsApp in RAM.*
