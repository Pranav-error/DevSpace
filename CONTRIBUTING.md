# Contributing to DevSpace

Thanks for your interest! DevSpace is a macOS menu bar app (Tauri v2, Rust + vanilla JS) for developers fighting RAM and disk pressure.

## Getting started

```bash
git clone https://github.com/Pranav-error/DevSpace.git
cd DevSpace
npm install
npm run tauri dev
```

Prereqs: Rust (rustup), Node.js, Xcode Command Line Tools. macOS only.

## Project layout

```
src/                  frontend (vanilla HTML/CSS/JS, no framework)
src-tauri/src/
  lib.rs              app wiring: tray, NSPanel popover, commands, poll loop
  scanner.rs          disk walker + 4-bucket classifier
  cleanup.rs          Trash-only cleanup + project hibernation
  archive.rs          checkpoint archiving (symlink-move, FP16/INT8 helper)
  docker.rs           docker CLI wrapper (surgical, never bulk)
  history.rs          SQLite: cleanup log + disk history + forecast
  alerts.rs           rate-limited notifications
  config.rs           ~/.devspace/config.json
docs/BETA-TESTING.md  manual QA checklist
```

## Rules that are not negotiable

These are the product's core safety guarantees. PRs that weaken them will be declined:

1. **Nothing is permanently deleted.** Destructive actions go to the macOS Trash (`trash` crate), never `rm -rf`/`remove_dir_all`.
2. **Always preview → confirm** with exact paths before anything destructive.
3. **Precious files (checkpoints, datasets, .env) are never deletable** through the UI — archive actions only.
4. **Nothing destructive is ever automatic.** No background cleanup, no auto-quantization.
5. Quitting other apps uses graceful ⌘Q semantics; never touch CLI/terminal processes.

## Development notes

- `cargo test` in `src-tauri/` must pass; add tests for classifier or forecast changes.
- The popover is a non-activating `NSPanel` (tauri-nspanel) — required to float over fullscreen Spaces. Don't replace it with a plain window; it will silently break for fullscreen users.
- `win.set_position` requires integer (physical-pixel) coordinates — f64 silently no-ops.
- Frontend has no build step: edit `src/*` directly. In dev mode changes hot-reload.
- Keep the UI within macOS-native visual language: quiet colors, hairlines, spring curves (`--spring` tokens in styles.css).

## Good first issues

- More classifier kinds (Go module cache, Maven `.m2`, Composer, Unity Library folders)
- Pure-Rust checkpoint conversion via [candle](https://github.com/huggingface/candle) to drop the Python dependency (big one, coordinate first)
- Per-volume selection UI for archiving
- Localization

## PR process

1. Fork, branch from `main`
2. Keep PRs focused — one feature/fix each
3. `cargo test` green, `node --check src/main.js` clean
4. Describe what you tested manually (see docs/BETA-TESTING.md for the relevant section)

## License

MIT — by contributing you agree your contributions are MIT-licensed.
