#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State,
};
use wakey_core::bot_bridge::ControlCommand;
use wakey_core::config::{Config, ScheduleRule, WeekdayShort};
use wakey_core::daemon::{start_daemon, DaemonHandle};
use wakey_core::habits::{default_history_path, detect_habits, load_records, HabitSuggestion};
use wakey_core::ipc::{DaemonSnapshot, IpcResponse};

struct AppState {
    daemon: DaemonHandle,
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> DaemonSnapshot {
    state.daemon.snapshot.lock().unwrap().clone()
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
fn save_config(cfg: Config) -> Result<(), String> {
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
    let parsed_days = WeekdayShort::parse_list(&days);
    if parsed_days.is_empty() {
        return Err(format!("invalid days '{days}'"));
    }
    let mut cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    cfg.schedules.retain(|s| s.name != name);
    cfg.schedules.push(ScheduleRule {
        name,
        days: parsed_days,
        start,
        end,
        keep_display_on,
        on_battery,
        enabled: true,
    });
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_habits() -> Result<Vec<HabitSuggestion>, String> {
    let cfg = Config::load_or_default().map_err(|e| e.to_string())?;
    let records = load_records(&default_history_path());
    Ok(detect_habits(&records, &cfg.habits))
}

fn main() {
    let daemon = start_daemon(true, true);

    tauri::Builder::default()
        .manage(AppState { daemon })
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "Show Dashboard", true, None::<&str>)?;
            let awake_1h = MenuItem::with_id(app, "awake_1h", "Awake 1h", true, None::<&str>)?;
            let awake_2h = MenuItem::with_id(app, "awake_2h", "Awake 2h", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Stop Manual Wakeup", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit wakey", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[&show, &awake_1h, &awake_2h, &stop, &quit])?;

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("wakey — Smart Keep-Awake");

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            tray_builder
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(win) = app.get_webview_window("main") {
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
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            send_command,
            get_config,
            save_config,
            add_schedule,
            get_habits,
        ])
        .run(tauri::generate_context!())
        .expect("error while running wakey dashboard");
}
