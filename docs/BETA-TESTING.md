# DevSpace — Beta Test Script

Automated coverage already in place (`cargo test`, 10 tests): classifier buckets,
.git pruning, rebuild-command detection, no-lockfile risk flag, dir sizing,
forecast regression (decline/stable/insufficient-data), cleaned-log totals,
Docker size parsing, app-bundle process grouping. The FP16 converter and its
crash-safety (corrupt input → original untouched, no backup litter) are verified
end-to-end against the real helper.

Everything below needs a human. Check items off as you go.

## Monitor tab
- [ ] Tray shows live text (`72% · 10G`) updating every ~3s
- [ ] Popover opens under the tray icon with spring animation; opens over **fullscreen** apps
- [ ] Click outside the popover → it closes; Esc also closes it
- [ ] Memory bar color: green normally, yellow near threshold, red above your RAM warn %
- [ ] Warning line appears under the bar when above threshold
- [ ] Click Memory section → expands to top-20 with inner scroll; chevron rotates; click again collapses
- [ ] Helper processes grouped ("Visual Studio Code · N processes"), sizes show `X GB · Y%`
- [ ] Hover an app row → Quit button; CLI rows (claude, node) have **no** Quit button
- [ ] Quit an app you don't mind closing (e.g. open Notes first) → confirm modal → app quits gracefully
- [ ] "Total saved with DevSpace" shows in the footer after any cleanup

## Disk tab
- [ ] Scan runs with live progress text; button disabled while scanning
- [ ] Re-scan while one is running → error toast, no crash
- [ ] Four groups appear color-dotted: Safe / Rebuildable / Large files / Precious
- [ ] Precious rows have NO checkbox — only Archive
- [ ] Venvs without a lockfile show the ⚠ flag
- [ ] Select-all per group; clicking anywhere on a row toggles it; selected rows highlight blue
- [ ] Clean button shows correct count + total; confirm modal lists exact paths
- [ ] After cleaning: items appear in **macOS Trash** (verify in Finder!), "Recently cleaned" section lists them
- [ ] Put a path in Settings → never-touch, try to clean it → error, file untouched
- [ ] Hibernate an idle project → regen dirs to Trash, `.devspace-hibernated.json` created
- [ ] Restore the project → rebuild command runs, marker removed

## ML tab
- [ ] Checkpoints grouped by project, biggest first, with totals
- [ ] Archive → FP16 on a real .pt → toast shows before → after (~50%)
- [ ] FP16 on a `.safetensors` → works; INT8 on `.safetensors` → not offered
- [ ] Plug in a USB drive → "Move to <drive> + symlink back" appears → file moves, symlink left behind, `torch.load` still works
- [ ] Unplug the drive → loading the symlink fails (expected); replug → works again

## Docker tab (start Docker Desktop first)
- [ ] Images split into dangling / unused / in-use; in-use have no Remove button
- [ ] Remove a dangling image → confirm → gone on refresh
- [ ] Stopped container removable; running container is not
- [ ] Build cache prune works
- [ ] Quit Docker → tab says Docker unavailable, no crash

## Settings tab
- [ ] Every field persists after Save (check `~/.devspace/config.json`)
- [ ] Lower RAM warn % below current usage → bar turns red within one poll + notification fires (once per 4h)
- [ ] Toggle "Live text in menu bar" off → tray text disappears
- [ ] Poll interval change takes effect (watch the tray update cadence)

## Tray / lifecycle
- [ ] Right-click menu: Show, Pause monitoring, Quit
- [ ] Pause → tray says "paused", stats freeze; Resume → updates return
- [ ] Quit → app fully exits (icon gone, no process)
- [ ] Relaunch from /Applications → state intact (total saved, config, history)
- [ ] Reboot Mac → (if added to Login Items) app returns

## Multi-day (leave running)
- [ ] After ~24h: forecast line changes from "collecting data (N/24)" to a real trend
- [ ] Disk-low notification when free < threshold (max once/24h)

## Known limitations (not bugs)
- FP16/INT8 needs torch in `~/.devspace/venv` (or system python3)
- `.gguf`/`.h5`/`.onnx`: detected, but only move-to-external applies
- Notifications use osascript (unbundled-safe); notarized builds could switch to the native API
- Volume sizes in Docker tab are not computed (docker API cost)
