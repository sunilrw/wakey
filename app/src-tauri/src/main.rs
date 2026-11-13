#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use serde::Serialize;
use tauri::{
    menu::{ContextMenu, Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State,
};
use wakey_core::bot_bridge::ControlCommand;
use wakey_core::config::{generate_passcode, Config, PairedPeer, ScheduleRule, WeekdayShort};
use wakey_core::daemon::{start_daemon, DaemonHandle};
use wakey_core::habits::{default_history_path, detect_habits, load_records, HabitSuggestion};
use wakey_core::ipc::{DaemonSnapshot, IpcResponse};
use wakey_core::peers::{
    fetch_paired_peers_status, format_pairing_clipboard, pair_with_remote,
    send_remote_peer_command, send_wake_on_lan, RemotePeerStatus,
};
use wakey_core::power::{
    clear_morning_wake as core_clear_morning_wake,
    schedule_morning_wake as core_schedule_morning_wake,
};

struct AppState {
    daemon: DaemonHandle,
    last_tray_rect: Mutex<Option<tauri::Rect>>,
}

fn position_window_under_tray(win: &tauri::WebviewWindow, rect: tauri::Rect) {
    let fallback_scale = win.scale_factor().unwrap_or(1.0);
    let fallback_scale = if fallback_scale > 0.0 && fallback_scale.is_finite() {
        fallback_scale
    } else {
        1.0
    };

    // First estimate using fallback scale to locate which monitor owns the tray icon.
    let est_pos = rect.position.to_physical::<i32>(fallback_scale);
    let est_size = rect.size.to_physical::<i32>(fallback_scale);
    let est_cx = est_pos.x + est_size.width / 2;
    let est_cy = est_pos.y + est_size.height / 2;

    let target_monitor = win
        .available_monitors()
        .ok()
        .and_then(|monitors| {
            monitors.into_iter().find(|m| {
                let p = m.position();
                let s = m.size();
                est_cx >= p.x
                    && est_cx < p.x + s.width as i32
                    && est_cy >= p.y
                    && est_cy < p.y + s.height as i32
            })
        })
        .or_else(|| win.current_monitor().ok().flatten())
        .or_else(|| win.primary_monitor().ok().flatten());

    let scale = target_monitor
        .as_ref()
        .map(|m| m.scale_factor())
        .filter(|s| *s > 0.0 && s.is_finite())
        .unwrap_or(fallback_scale);

    let icon_pos = rect.position.to_physical::<i32>(scale);
    let icon_size = rect.size.to_physical::<i32>(scale);
    let outer_size = win.outer_size().ok();
    let win_w = outer_size
        .map(|s| s.width as i32)
        .unwrap_or_else(|| (395.0 * scale) as i32);
    let win_h = outer_size
        .map(|s| s.height as i32)
        .unwrap_or_else(|| (740.0 * scale) as i32);

    let icon_center_x = icon_pos.x + icon_size.width / 2;
    let icon_bottom_y = icon_pos.y + icon_size.height;
    let gap = (4.0 * scale) as i32;

    let mut x = icon_center_x - win_w / 2;
    let mut y = icon_bottom_y + gap;

    if let Some(monitor) = target_monitor {
        let mon_pos = monitor.position();
        let mon_size = monitor.size();
        let min_x = mon_pos.x + 8;
        let max_x = (mon_pos.x + mon_size.width as i32 - win_w - 8).max(min_x);
        x = x.clamp(min_x, max_x);

        let mon_top = mon_pos.y + 8;
        let mon_bottom = mon_pos.y + mon_size.height as i32 - 8;
        if y + win_h > mon_bottom {
            // Bottom-anchored taskbar (e.g. Windows): place popover above the tray icon.
            y = icon_pos.y - win_h - gap;
        }
        let max_y = (mon_bottom - win_h).max(mon_top);
        y = y.clamp(mon_top, max_y);
    } else {
        x = x.max(10);
        y = y.max(10);
    }

    let _ = win.set_position(tauri::Position::Physical(
        tauri::PhysicalPosition::new(x, y),
    ));
}

fn is_valid_hhmm(s: &str) -> bool {
    let Some((hh, mm)) = s.trim().split_once(':') else {
        return false;
    };
    hh.len() == 2
        && mm.len() == 2
        && matches!((hh.parse::<u8>(), mm.parse::<u8>()), (Ok(h), Ok(m)) if h < 24 && m < 60)
}

fn validate_http_url_or_empty(url: &str, field: &str) -> Result<String, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
        && !trimmed.chars().any(char::is_whitespace)
    {
        Ok(trimmed.to_string())
    } else {
        Err(format!("{field} must start with http:// or https://"))
    }
}

fn compute_max_popover_height(win: &tauri::WebviewWindow) -> f64 {
    win.current_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let mon_h_logical = m.size().height as f64 / m.scale_factor().max(1.0);
            (mon_h_logical - 48.0).max(160.0)
        })
        .unwrap_or(900.0)
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> DaemonSnapshot {
    state
        .daemon
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[tauri::command]
fn send_command(state: State<'_, AppState>, cmd: ControlCommand) -> Result<IpcResponse, String> {
    state.daemon.send_command(cmd)
}

#[tauri::command]
fn get_config() -> Result<Config, String> {
    Config::load_or_default().map_err(|e| e.to_string())
}

#[tauri::command]
fn save_config(mut cfg: Config) -> Result<(), String> {
    if !(5..=95).contains(&cfg.general.min_battery_percent) {
        return Err("min_battery_percent must be between 5 and 95".into());
    }
    cfg.peers.relay_url = validate_http_url_or_empty(&cfg.peers.relay_url, "relay_url")?;
    cfg.bot.webhook_url = validate_http_url_or_empty(&cfg.bot.webhook_url, "webhook_url")?;
    for sched in &cfg.schedules {
        if sched.name.trim().is_empty() {
            return Err("schedule name cannot be empty".into());
        }
        if sched.days.is_empty() {
            return Err(format!("schedule '{}' must have at least one day", sched.name));
        }
        if !is_valid_hhmm(&sched.start) || !is_valid_hhmm(&sched.end) {
            return Err(format!(
                "schedule '{}' must use HH:MM start/end times",
                sched.name
            ));
        }
    }

    // Preserve concurrent peer identity/pairing mutations made outside the settings form.
    if let Ok(existing) = Config::load_or_default() {
        cfg.peers.device_id = existing.peers.device_id;
        cfg.peers.passcode = existing.peers.passcode;
        cfg.peers.paired_peers = existing.peers.paired_peers;
    }

    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn add_schedule(
    name: String,
    days: String,
    start: String,
    end: String,
    keep_display_on: bool,
    on_battery: bool,
) -> Result<(), String> {
    let trimmed_name = name.trim().to_string();
    if trimmed_name.is_empty() {
        return Err("schedule name cannot be empty".into());
    }
    if !is_valid_hhmm(&start) || !is_valid_hhmm(&end) {
        return Err("start and end must use HH:MM (24-hour) format".into());
    }
    let parsed_days = WeekdayShort::parse_list(&days);
    if parsed_days.is_empty() {
        return Err(format!("invalid days '{days}'"));
    }
    let mut cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    cfg.schedules.retain(|s| s.name != trimmed_name);
    cfg.schedules.push(ScheduleRule {
        name: trimmed_name,
        days: parsed_days,
        start: start.trim().to_string(),
        end: end.trim().to_string(),
        keep_display_on,
        on_battery,
        enabled: true,
    });
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn schedule_morning_wake(days: String, clock: String) -> Result<String, String> {
    let parsed_days = WeekdayShort::parse_list(&days);
    if parsed_days.is_empty() {
        return Err(format!("invalid days '{days}'"));
    }
    let trimmed_clock = clock.trim();
    if !is_valid_hhmm(trimmed_clock) {
        return Err("wake time must use HH:MM (24-hour) format".into());
    }
    core_schedule_morning_wake(&parsed_days, trimmed_clock)
}

#[tauri::command]
fn clear_morning_wake() -> Result<String, String> {
    core_clear_morning_wake()
}

#[derive(Serialize)]
struct PeerIdentityResponse {
    device_id: String,
    passcode: String,
    device_name: String,
    pairing_code: String,
    relay_url: String,
}

#[tauri::command]
fn get_peer_identity() -> Result<PeerIdentityResponse, String> {
    let cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    Ok(PeerIdentityResponse {
        pairing_code: format_pairing_clipboard(&cfg.peers),
        device_id: cfg.peers.device_id,
        passcode: cfg.peers.passcode,
        device_name: cfg.peers.device_name,
        relay_url: cfg.peers.relay_url,
    })
}

#[tauri::command]
fn regenerate_passcode() -> Result<PeerIdentityResponse, String> {
    let mut cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    cfg.peers.passcode = generate_passcode();
    cfg.save().map_err(|e| e.to_string())?;
    Ok(PeerIdentityResponse {
        pairing_code: format_pairing_clipboard(&cfg.peers),
        device_id: cfg.peers.device_id,
        passcode: cfg.peers.passcode,
        device_name: cfg.peers.device_name,
        relay_url: cfg.peers.relay_url,
    })
}

#[tauri::command]
fn set_relay_url(relay_url: String, device_name: Option<String>) -> Result<PeerIdentityResponse, String> {
    let validated_url = validate_http_url_or_empty(&relay_url, "relay_url")?;
    let mut cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    cfg.peers.relay_url = validated_url;
    if let Some(name) = device_name {
        if !name.trim().is_empty() {
            cfg.peers.device_name = name.trim().to_string();
        }
    }
    cfg.save().map_err(|e| e.to_string())?;
    Ok(PeerIdentityResponse {
        pairing_code: format_pairing_clipboard(&cfg.peers),
        device_id: cfg.peers.device_id,
        passcode: cfg.peers.passcode,
        device_name: cfg.peers.device_name,
        relay_url: cfg.peers.relay_url,
    })
}

#[tauri::command]
fn pair_peer(code: String, passcode: Option<String>) -> Result<PairedPeer, String> {
    let mut cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    pair_with_remote(&mut cfg, &code, passcode.as_deref())
}

#[tauri::command]
fn remove_peer(device_id: String) -> Result<(), String> {
    let mut cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    cfg.peers.paired_peers.retain(|p| p.device_id != device_id);
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_peers_status() -> Result<Vec<RemotePeerStatus>, String> {
    let cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    Ok(fetch_paired_peers_status(&cfg))
}

#[tauri::command]
fn send_peer_command(device_id: String, cmd: ControlCommand) -> Result<String, String> {
    let cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    send_remote_peer_command(&cfg, &device_id, &cmd)
}

#[tauri::command]
fn wake_peer_lan(mac: String) -> Result<(), String> {
    send_wake_on_lan(&mac)
}

#[tauri::command]
fn get_habits() -> Result<Vec<HabitSuggestion>, String> {
    let cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    let records = load_records(&default_history_path());
    Ok(detect_habits(&records, &cfg.habits))
}

#[tauri::command]
fn hide_popover(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

#[tauri::command]
fn set_popover_size(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    wide: bool,
    height: Option<f64>,
) {
    if let Some(win) = app.get_webview_window("main") {
        let w = if wide { 600.0 } else { 395.0 };
        let scale = win.scale_factor().unwrap_or(1.0);
        let scale = if scale > 0.0 && scale.is_finite() {
            scale
        } else {
            1.0
        };

        let max_h = compute_max_popover_height(&win);
        let cur_h = win
            .outer_size()
            .map(|s| s.height as f64 / scale)
            .unwrap_or(740.0);
        let h = height
            .filter(|h| h.is_finite())
            .unwrap_or(cur_h)
            .clamp(160.0, max_h);

        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(w, h)));
        if let Some(rect) = *state
            .last_tray_rect
            .lock()
            .unwrap_or_else(|e| e.into_inner())
        {
            position_window_under_tray(&win, rect);
        }
    }
}

#[tauri::command]
fn fit_popover_height(app: tauri::AppHandle, height: f64, wide: bool) {
    if !height.is_finite() {
        return;
    }
    if let Some(win) = app.get_webview_window("main") {
        let w = if wide { 600.0 } else { 395.0 };
        let max_h = compute_max_popover_height(&win);
        let h = height.clamp(160.0, max_h);
        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(w, h)));
    }
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

fn main() {
    let daemon = start_daemon(true, true);

    tauri::Builder::default()
        .manage(AppState {
            daemon,
            last_tray_rect: Mutex::new(None),
        })
        .setup(|app| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.hide();
            }

            let show = MenuItem::with_id(app, "show", "Show Popover", true, None::<&str>)?;
            let awake_1h = MenuItem::with_id(app, "awake_1h", "Awake 1h", true, None::<&str>)?;
            let awake_2h = MenuItem::with_id(app, "awake_2h", "Awake 2h", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Stop / Allow Sleep", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit wakey", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[&show, &awake_1h, &awake_2h, &stop, &quit])?;

            // On macOS an attached NSMenu intercepts left-clicks, so we popup on right-click.
            // On Windows/Linux, attach `.menu(&menu)` with `show_menu_on_left_click(false)` so
            // right-click works natively even when the popover window is hidden.
            let mut tray_builder = TrayIconBuilder::new().tooltip("wakey — Smart Keep-Awake");

            #[cfg(not(target_os = "macos"))]
            {
                tray_builder = tray_builder.menu(&menu).show_menu_on_left_click(false);
            }

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            tray_builder
                .on_tray_icon_event(move |tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button,
                        button_state: tauri::tray::MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        let app_handle = tray.app_handle();
                        if let Some(state) = app_handle.try_state::<AppState>() {
                            *state
                                .last_tray_rect
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) = Some(rect);
                        }
                        if let Some(win) = app_handle.get_webview_window("main") {
                            match button {
                                tauri::tray::MouseButton::Left => {
                                    if win.is_visible().unwrap_or(false) {
                                        let _ = win.hide();
                                    } else {
                                        position_window_under_tray(&win, rect);
                                        let _ = win.show();
                                        position_window_under_tray(&win, rect);
                                        let _ = win.set_focus();
                                        let win_delayed = win.clone();
                                        std::thread::spawn(move || {
                                            std::thread::sleep(std::time::Duration::from_millis(35));
                                            position_window_under_tray(&win_delayed, rect);
                                        });
                                    }
                                }
                                #[cfg(target_os = "macos")]
                                tauri::tray::MouseButton::Right => {
                                    let _ = menu.popup(win.as_ref().window().clone());
                                }
                                _ => {}
                            }
                        }
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(win) = app.get_webview_window("main") {
                            if let Some(rect) = *app
                                .state::<AppState>()
                                .last_tray_rect
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                            {
                                position_window_under_tray(&win, rect);
                            }
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                    "awake_1h" => {
                        let state = app.state::<AppState>();
                        let _ = state.daemon.send_command(ControlCommand::AwakeFor {
                            duration: "1h".into(),
                            display: false,
                        });
                    }
                    "awake_2h" => {
                        let state = app.state::<AppState>();
                        let _ = state.daemon.send_command(ControlCommand::AwakeFor {
                            duration: "2h".into(),
                            display: false,
                        });
                    }
                    "stop" => {
                        let state = app.state::<AppState>();
                        let _ = state.daemon.send_command(ControlCommand::Stop);
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            send_command,
            get_config,
            save_config,
            add_schedule,
            schedule_morning_wake,
            clear_morning_wake,
            get_habits,
            get_peer_identity,
            regenerate_passcode,
            set_relay_url,
            pair_peer,
            remove_peer,
            get_peers_status,
            send_peer_command,
            wake_peer_lan,
            hide_popover,
            set_popover_size,
            fit_popover_height,
            quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running wakey dashboard");
}
