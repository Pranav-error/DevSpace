mod alerts;
mod archive;
mod cleanup;
mod config;
mod docker;
mod history;
mod scanner;

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use sysinfo::{Disks, ProcessesToUpdate, System};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, State,
};
use tauri_nspanel::{
    cocoa::appkit::NSWindowCollectionBehavior, panel_delegate, ManagerExt, WebviewWindowExt,
};

use alerts::Alerter;
use config::Config;
use history::Db;
use scanner::ScanResult;

// ---------- state ----------

struct PopoverTimes {
    shown_at: Instant,
    hidden_at: Instant,
}
struct PopoverState(Mutex<PopoverTimes>);

struct AppState {
    config: Mutex<Config>,
    paused: AtomicBool,
    scanning: AtomicBool,
    last_scan: Mutex<Option<ScanResult>>,
}

struct TrayHandles {
    tray: Mutex<Option<TrayIcon>>,
    pause_item: Mutex<Option<MenuItem<tauri::Wry>>>,
}

// ---------- stats ----------

#[derive(Clone, serde::Serialize)]
struct ProcStat {
    /// App name (helpers grouped under their .app bundle) or process name.
    name: String,
    memory_bytes: u64,
    process_count: u32,
    pids: Vec<u32>,
    /// True when the name comes from a .app bundle — only those get a
    /// Quit button (graceful ⌘Q). CLI processes could be someone's live
    /// terminal session; killing them blind is a footgun.
    is_app: bool,
}

/// "/Applications/Visual Studio Code.app/.../Code Helper" → "Visual Studio Code"
fn app_name_from_path(path: &std::path::Path) -> Option<String> {
    path.components().find_map(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.strip_suffix(".app").map(|name| name.to_string())
    })
}

#[derive(Clone, serde::Serialize)]
struct Stats {
    memory_used_bytes: u64,
    memory_total_bytes: u64,
    top_processes: Vec<ProcStat>,
    disk_free_bytes: u64,
    disk_total_bytes: u64,
    paused: bool,
    total_saved_bytes: u64,
}

fn collect_stats(sys: &mut System) -> (Stats, f64) {
    sys.refresh_memory();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    // Group helper processes under their parent app so the list reads
    // "Visual Studio Code · 12 processes" instead of "Code Helper (Renderer)".
    let mut by_app: std::collections::HashMap<(String, bool), (u64, Vec<u32>)> =
        std::collections::HashMap::new();
    for p in sys.processes().values() {
        let app_name = p.exe().and_then(app_name_from_path);
        let is_app = app_name.is_some();
        let name = app_name.unwrap_or_else(|| p.name().to_string_lossy().into_owned());
        let entry = by_app.entry((name, is_app)).or_default();
        entry.0 += p.memory();
        entry.1.push(p.pid().as_u32());
    }
    let mut procs: Vec<ProcStat> = by_app
        .into_iter()
        .map(|((name, is_app), (mem, pids))| ProcStat {
            name,
            memory_bytes: mem,
            process_count: pids.len() as u32,
            pids,
            is_app,
        })
        .collect();
    procs.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));
    // Frontend shows 5 collapsed / 20 when the memory section is expanded.
    procs.truncate(20);

    let disks = Disks::new_with_refreshed_list();
    let root = disks
        .iter()
        .find(|d| d.mount_point().to_str() == Some("/"))
        .or_else(|| disks.iter().next());
    let (disk_free, disk_total) = root
        .map(|d| (d.available_space(), d.total_space()))
        .unwrap_or((0, 0));

    let mem_pct = if sys.total_memory() > 0 {
        sys.used_memory() as f64 / sys.total_memory() as f64 * 100.0
    } else {
        0.0
    };

    (
        Stats {
            memory_used_bytes: sys.used_memory(),
            memory_total_bytes: sys.total_memory(),
            top_processes: procs,
            disk_free_bytes: disk_free,
            disk_total_bytes: disk_total,
            paused: false,
            total_saved_bytes: 0,
        },
        mem_pct,
    )
}

// ---------- tray icon drawing ----------

/// A simple gauge glyph (ring + needle) rendered as a template image so
/// macOS recolors it for light/dark menu bars.
fn tray_icon_image() -> tauri::image::Image<'static> {
    const S: usize = 44; // 22pt @2x
    let c = (S as f64 - 1.0) / 2.0;
    let ring_r = 16.0;
    let thick = 3.0;
    // Needle points up-right (45°).
    let (nx, ny) = (std::f64::consts::FRAC_1_SQRT_2, -std::f64::consts::FRAC_1_SQRT_2);
    let needle_len = ring_r - 5.0;

    let mut rgba = vec![0u8; S * S * 4];
    for y in 0..S {
        for x in 0..S {
            let dx = x as f64 - c;
            let dy = y as f64 - c;
            let r = (dx * dx + dy * dy).sqrt();

            let ring_a = (1.0 - ((r - ring_r).abs() / thick)).clamp(0.0, 1.0);

            // Distance from the pixel to the needle segment (0..needle_len).
            let t = (dx * nx + dy * ny).clamp(0.0, needle_len);
            let (px, py) = (t * nx, t * ny);
            let seg_d = ((dx - px).powi(2) + (dy - py).powi(2)).sqrt();
            let needle_a = (1.0 - (seg_d / 2.2)).clamp(0.0, 1.0);

            let hub_a = (1.0 - ((r - 2.5).max(0.0) / 1.5)).clamp(0.0, 1.0);

            let a = ring_a.max(needle_a).max(hub_a);
            let i = (y * S + x) * 4;
            rgba[i] = 0;
            rgba[i + 1] = 0;
            rgba[i + 2] = 0;
            rgba[i + 3] = (a * 255.0) as u8;
        }
    }
    tauri::image::Image::new_owned(rgba, S as u32, S as u32)
}

// ---------- panel ----------

fn init_panel(app: &AppHandle) {
    let win = app.get_webview_window("popover").unwrap();
    let panel = win.to_panel().unwrap();

    panel.set_level(25); // NSStatusWindowLevel

    #[allow(non_upper_case_globals)]
    const NSWindowStyleMaskNonActivatingPanel: i32 = 1 << 7;
    panel.set_style_mask(NSWindowStyleMaskNonActivatingPanel);

    panel.set_collection_behaviour(
        NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary
            | NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces,
    );

    let delegate = panel_delegate!(PopoverPanelDelegate {
        window_did_resign_key
    });
    let handle = app.clone();
    delegate.set_listener(Box::new(move |delegate_name: String| {
        if delegate_name == "window_did_resign_key" {
            let state = handle.state::<PopoverState>();
            let mut times = state.0.lock().unwrap();
            if times.shown_at.elapsed() > Duration::from_millis(500) {
                times.hidden_at = Instant::now();
                if let Ok(panel) = handle.get_webview_panel("popover") {
                    panel.order_out(None);
                }
            }
        }
    }));
    panel.set_delegate(delegate);
}

fn show_popover(app: &AppHandle, tray_rect: Option<tauri::Rect>) {
    let Some(win) = app.get_webview_window("popover") else {
        return;
    };
    if let Some(rect) = tray_rect {
        let scale = win.scale_factor().unwrap_or(1.0);
        let pos = match rect.position {
            tauri::Position::Physical(p) => (p.x as f64, p.y as f64),
            tauri::Position::Logical(p) => (p.x * scale, p.y * scale),
        };
        let size = match rect.size {
            tauri::Size::Physical(s) => (s.width as f64, s.height as f64),
            tauri::Size::Logical(s) => (s.width * scale, s.height * scale),
        };
        let win_width = win.outer_size().map(|s| s.width as f64).unwrap_or(0.0);
        // set_position needs integer pixels; f64 coordinates are silently ignored
        let x = (pos.0 + size.0 / 2.0 - win_width / 2.0) as i32;
        let y = (pos.1 + size.1 + 6.0 * scale) as i32;
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
    if let Some(state) = app.try_state::<PopoverState>() {
        state.0.lock().unwrap().shown_at = Instant::now();
    }
    if let Ok(panel) = app.get_webview_panel("popover") {
        panel.show();
        // Let the frontend replay its entrance animation.
        let _ = app.emit("popover-shown", ());
    }
}

fn toggle_popover(app: &AppHandle, tray_rect: Option<tauri::Rect>) {
    if let Ok(panel) = app.get_webview_panel("popover") {
        if panel.is_visible() {
            panel.order_out(None);
            return;
        }
    }
    if let Some(state) = app.try_state::<PopoverState>() {
        if state.0.lock().unwrap().hidden_at.elapsed() < Duration::from_millis(400) {
            return;
        }
    }
    show_popover(app, tray_rect);
}

// ---------- commands ----------

/// Ask the app to quit gracefully (like ⌘Q); fall back to SIGTERM.
#[tauri::command]
fn quit_app(name: String, pids: Vec<u32>) -> Result<(), String> {
    let script = format!("tell application \"{}\" to quit", name.replace('"', ""));
    let graceful = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !graceful {
        for pid in pids {
            let _ = std::process::Command::new("kill")
                .arg("-15")
                .arg(pid.to_string())
                .status();
        }
    }
    Ok(())
}

#[tauri::command]
fn resize_popover(app: AppHandle, height: f64) {
    if let Some(win) = app.get_webview_window("popover") {
        let _ = win.set_size(tauri::LogicalSize::new(340.0, height));
    }
}

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(state: State<AppState>, config: Config) -> Result<(), String> {
    config::save(&config)?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
fn set_paused(state: State<AppState>, paused: bool) {
    state.paused.store(paused, Ordering::Relaxed);
}

#[tauri::command]
fn start_scan(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err("scan already running".into());
    }
    let cfg = state.config.lock().unwrap().clone();
    let handle = app.clone();
    thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scanner::scan(&cfg, |path, visited| {
                let _ = handle.emit("scan-progress", serde_json::json!({ "path": path, "visited": visited }));
            })
        }));
        let state = handle.state::<AppState>();
        state.scanning.store(false, Ordering::SeqCst);
        match outcome {
            Ok(result) => {
                *state.last_scan.lock().unwrap() = Some(result.clone());
                let _ = handle.emit("scan-done", &result);
            }
            Err(_) => {
                let _ = handle.emit("scan-error", "scan crashed — check scan roots in Settings");
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn clean_paths(app: AppHandle, paths: Vec<String>) -> cleanup::CleanOutcome {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();
    let db = app.state::<Db>();
    cleanup::clean_paths(&cfg, &db, &paths)
}

#[tauri::command]
fn recently_cleaned(db: State<Db>) -> Vec<history::CleanedEntry> {
    db.recently_cleaned()
}

#[tauri::command]
async fn docker_info() -> docker::DockerInfo {
    docker::info()
}

#[tauri::command]
async fn docker_remove(kind: String, id: String) -> Result<String, String> {
    docker::remove(&kind, &id)
}

#[tauri::command]
fn list_volumes() -> Vec<String> {
    archive::list_volumes()
}

#[tauri::command]
async fn archive_move(path: String, volume: String) -> Result<archive::ArchiveOutcome, String> {
    archive::move_and_symlink(&path, &volume)
}

#[tauri::command]
async fn archive_convert(path: String, mode: String) -> Result<archive::ArchiveOutcome, String> {
    archive::convert(&path, &mode)
}

#[tauri::command]
fn hibernate_project(app: AppHandle, root: String) -> Result<cleanup::HibernateOutcome, String> {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();
    let scan = state.last_scan.lock().unwrap();
    let scan = scan.as_ref().ok_or("run a scan first")?;
    let regen_low: Vec<String> = scan
        .findings
        .iter()
        .filter(|f| f.bucket == scanner::Bucket::RegenLow)
        .map(|f| f.path.clone())
        .collect();
    let rebuild = scan
        .projects
        .iter()
        .find(|p| p.root == root)
        .and_then(|p| p.rebuild_cmd.clone());
    let db = app.state::<Db>();
    cleanup::hibernate_project(&cfg, &db, &root, &regen_low, rebuild.as_deref())
}

#[tauri::command]
async fn restore_project(root: String) -> Result<cleanup::RestoreOutcome, String> {
    cleanup::restore_project(&root)
}

#[tauri::command]
fn get_forecast(app: AppHandle) -> history::Forecast {
    let state = app.state::<AppState>();
    let threshold = state.config.lock().unwrap().disk_warn_gb;
    let disks = Disks::new_with_refreshed_list();
    let free = disks
        .iter()
        .find(|d| d.mount_point().to_str() == Some("/"))
        .map(|d| d.available_space())
        .unwrap_or(0);
    app.state::<Db>().forecast(free, threshold)
}

// ---------- poll loop ----------

fn spawn_poll_loop(app: AppHandle) {
    thread::spawn(move || {
        let mut sys = System::new();
        let mut last_forecast_check = Instant::now() - Duration::from_secs(3600);
        loop {
            let (interval, paused, warn_pct, warn_gb, live_text) = {
                let state = app.state::<AppState>();
                let cfg = state.config.lock().unwrap();
                (
                    cfg.poll_interval_secs.max(1),
                    state.paused.load(Ordering::Relaxed),
                    cfg.mem_warn_pct,
                    cfg.disk_warn_gb,
                    cfg.tray_live_text,
                )
            };
            if paused {
                thread::sleep(Duration::from_secs(1));
                continue;
            }

            let (mut stats, mem_pct) = collect_stats(&mut sys);
            let db = app.state::<Db>();
            stats.total_saved_bytes = db.total_saved();
            db.log_disk_free(stats.disk_free_bytes);

            const GB: f64 = 1024.0 * 1024.0 * 1024.0;
            let free_gb = stats.disk_free_bytes as f64 / GB;

            // Live tray text.
            if let Some(trays) = app.try_state::<TrayHandles>() {
                if let Some(tray) = trays.tray.lock().unwrap().as_ref() {
                    let title = if live_text {
                        Some(format!("{:.0}% · {:.0}G", mem_pct, free_gb))
                    } else {
                        None
                    };
                    let _ = tray.set_title(title);
                }
            }

            // Alerts (rate-limited).
            let alerter = app.state::<Alerter>();
            if mem_pct >= warn_pct {
                alerter.fire(
                    &app,
                    "mem",
                    Duration::from_secs(4 * 3600),
                    "Memory pressure high",
                    &format!("RAM at {mem_pct:.0}% — consider closing some apps"),
                );
            }
            if free_gb < warn_gb {
                alerter.fire(
                    &app,
                    "disk",
                    Duration::from_secs(24 * 3600),
                    "Disk space low",
                    &format!("Only {free_gb:.1} GB free — run a DevSpace scan"),
                );
            }
            if last_forecast_check.elapsed() > Duration::from_secs(3600) {
                last_forecast_check = Instant::now();
                let fc = db.forecast(stats.disk_free_bytes, warn_gb);
                if let Some(days) = fc.days_left {
                    if days < 14.0 {
                        alerter.fire(
                            &app,
                            "forecast",
                            Duration::from_secs(24 * 3600),
                            "Disk filling up",
                            &format!("At the current rate you'll hit {warn_gb:.0} GB free in ~{days:.0} days"),
                        );
                    }
                }
            }

            let _ = app.emit("stats", &stats);
            thread::sleep(Duration::from_secs(interval));
        }
    });
}

// ---------- entry ----------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_nspanel::init())
        .invoke_handler(tauri::generate_handler![
            resize_popover,
            quit_app,
            get_config,
            save_config,
            set_paused,
            start_scan,
            clean_paths,
            recently_cleaned,
            docker_info,
            docker_remove,
            list_volumes,
            archive_move,
            archive_convert,
            hibernate_project,
            restore_project,
            get_forecast,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let boot = Instant::now();
            app.manage(PopoverState(Mutex::new(PopoverTimes {
                shown_at: boot,
                hidden_at: boot,
            })));
            app.manage(AppState {
                config: Mutex::new(config::load()),
                paused: AtomicBool::new(false),
                scanning: AtomicBool::new(false),
                last_scan: Mutex::new(None),
            });
            app.manage(history::open());
            app.manage(Alerter::new());

            init_panel(app.handle());

            let show = MenuItem::with_id(app, "show", "Show DevSpace", true, None::<&str>)?;
            let pause = MenuItem::with_id(app, "pause", "Pause monitoring", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &pause, &quit])?;

            let tray = TrayIconBuilder::with_id("devspace-tray")
                .icon(tray_icon_image())
                .icon_as_template(true)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_popover(app, None),
                    "pause" => {
                        let state = app.state::<AppState>();
                        let now_paused = !state.paused.load(Ordering::Relaxed);
                        state.paused.store(now_paused, Ordering::Relaxed);
                        let trays = app.state::<TrayHandles>();
                        if let Some(item) = trays.pause_item.lock().unwrap().as_ref() {
                            let _ = item.set_text(if now_paused {
                                "Resume monitoring"
                            } else {
                                "Pause monitoring"
                            });
                        }
                        if now_paused {
                            if let Some(t) = trays.tray.lock().unwrap().as_ref() {
                                let _ = t.set_title(Some("paused"));
                            }
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        toggle_popover(tray.app_handle(), Some(rect));
                    }
                })
                .build(app)?;

            app.manage(TrayHandles {
                tray: Mutex::new(Some(tray)),
                pause_item: Mutex::new(Some(pause)),
            });

            spawn_poll_loop(app.handle().clone());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
