# DevSpace

**A macOS menu bar app for developers whose Macs keep running out of RAM and disk.**

DevSpace lives in your menu bar and shows live memory + disk stats at a glance. Click it and you get a native-feeling popover that can scan your dev folders, tell the difference between junk you can safely delete and ML checkpoints you'd cry about losing, surgically clean Docker, and forecast when your disk will run out.

Built with **Tauri v2** (Rust + vanilla JS) — idles under 10 MB of RAM, no Electron.

<p align="center">
  <em>Menu bar: <code>72% · 10G</code> — click for the popover</em>
</p>

---

## Why another cleaner?

ClearDisk, DevClean, and friends all do generic dev-cache cleanup. DevSpace's angle:

1. **ML-aware** — it knows `node_modules` is disposable but `.pt` / `.safetensors` / `.ckpt` checkpoints and datasets are precious. Precious files are *never* deletable in the UI, only archivable.
2. **Unified RAM + disk** — one tool, both problems, one menu bar icon.
3. **Surgical Docker cleanup** — dangling vs unused vs in-use images, per-item removal. No nuke-everything button.
4. **Checkpoint archiving** — move to an external drive with a symlink back (training scripts keep working), or compress via FP16/INT8 quantization.

## Features

### Monitor (live, every 3s)
- Memory bar with configurable warning threshold, top 5 processes, disk free
- Live text in the menu bar (`72% · 10G`), toggleable
- Storage forecasting: logs hourly disk samples to SQLite, fits a recency-weighted linear trend, warns when you'll hit your low-disk threshold ("~18 days")
- Rate-limited macOS notifications for high RAM, low disk, and bad forecasts

### Disk scanner (on demand)
Walks your configured roots and classifies everything into buckets:

| Bucket | Examples | UI treatment |
|---|---|---|
| 🟢 Safe to clean | `node_modules`, `dist`, `.next`, `__pycache__`, `target`, DerivedData, npm/pip/cargo/gradle caches | checkbox, one-click select-all |
| 🟡 Rebuildable | `.venv`, conda envs, ollama/HF models, VSCode extensions — flagged ⚠ if no lockfile exists to rebuild from | checkbox, warned |
| 🔵 Large files | anything ≥500 MB that isn't a dev artifact | checkbox |
| 🟣 Precious | `.pt` `.pth` `.ckpt` `.safetensors` `.gguf` `.h5` `.onnx`, big `.bin`, `.env` files | **no checkbox — archive only** |

### Safe cleanup
- Everything goes to the **Trash** (`trash` crate), never `rm -rf`
- Preview-then-confirm modal with exact paths, every time
- 24h "recently cleaned" undo list + cumulative "Total saved" counter (SQLite)

### Docker
- Images split into dangling / unused / **in-use (protected)**
- Per-image and per-container removal, build-cache prune — nothing bulk

### Checkpoint archiver
Per-file, always user-triggered, size preview first:
- **Move to external volume + symlink back** — lossless, killer feature for training scripts
- **FP16 in place** — ~50% smaller, ~zero accuracy loss (via a bundled Python helper, needs `torch`)
- **INT8 in place** — ~75% smaller, clearly flagged as lossy / not bit-identical

### Project hibernation
- Tracks last-touched time per project; suggests hibernating ones idle 60+ days
- Hibernate = trash regenerable content + write a `.devspace-hibernated.json` marker recording how to rebuild (`npm install`, `pip install -r requirements.txt`, …)
- One-click restore runs the rebuild command

## Install / build

Prereqs: Rust (rustup), Node.js, Xcode Command Line Tools.

```bash
git clone https://github.com/Pranav-error/DevSpace.git
cd DevSpace
npm install
npm run tauri dev      # run in dev mode
npm run tauri build    # produce the standalone .app
```

Config lives at `~/.devspace/config.json` (scan roots, thresholds, ignore globs, never-touch paths) and is editable from the Settings tab.

## Architecture

Five parts, two clocks:

1. **Shell** — tray icon + a non-activating `NSPanel` popover (via [tauri-nspanel](https://github.com/ahkohd/tauri-nspanel)) so it floats over fullscreen apps, with native vibrancy and spring animations
2. **Memory engine** — continuous 3s `sysinfo` poll, read-only
3. **Disk engine** — on-demand background scan with a 3-bucket classifier
4. **Rules/config** — shared JSON both engines read
5. **Alerts** — cross-cutting, rate-limited notifications

The two engines run on different clocks (continuous vs on-demand) by design.

## Safety principles

- Destructive actions go to the **Trash**, never permanent delete
- Always **preview → confirm** with exact paths
- **Precious files can't be deleted** through the UI at all
- Quantization is never automatic and always labeled lossy

## Stack

- [Tauri v2](https://tauri.app) — Rust backend, macOS system webview (~10 MB idle vs ~300 MB for Electron)
- [sysinfo](https://crates.io/crates/sysinfo) · [walkdir](https://crates.io/crates/walkdir) · [trash](https://crates.io/crates/trash) · [rusqlite](https://crates.io/crates/rusqlite) · [tauri-nspanel](https://github.com/ahkohd/tauri-nspanel)
- Vanilla HTML/CSS/JS frontend — no framework, spring physics via CSS `linear()` curves

## Roadmap

- [x] v1 — menu bar live readout
- [x] v1.5 — live tray text, custom template icon, pause/resume
- [x] v2 — disk scanner + classifier
- [x] v3 — Trash-only cleanup with undo window
- [x] v4 — Docker surgical cleanup + checkpoint archiver
- [x] v5 — hibernation + storage forecasting
- [ ] Notarized releases
- [ ] FSEvents-based hibernation auto-restore
- [ ] Per-project growth forecasting

---

*Personal project by a CS student who got tired of deleting Docker to fit WhatsApp in RAM.*
