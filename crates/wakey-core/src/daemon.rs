use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, Timelike};

use crate::agent_watch::AgentWatcher;
use crate::bot_bridge::{
    send_bot_event_async, spawn_http_bridge, BotEventPayload, ControlCommand,
};
use crate::config::{Config, WeekdayShort};
use crate::habits::{
    append_record, default_history_path, load_records, matching_habit_now, SessionRecord,
};
use crate::ipc::{spawn_ipc_server, AuditEntry, BatteryInfo, DaemonSnapshot, IpcResponse};
use crate::notify::send_desktop_notification;
use crate::power::{battery_status, PowerGuard, PowerOptions};
use crate::scheduler::{evaluate_schedules, ScheduleState};
use crate::time_parse::{duration_until_clock, parse_duration};

struct ManualSession {
    ends_at: Option<Instant>,
    total_duration_mins: u32,
    keep_display_on: bool,
    warned_pre_expiry: bool,
}

pub struct DaemonHandle {
    pub cmd_tx: Sender<(ControlCommand, Sender<String>)>,
    pub snapshot: Arc<Mutex<DaemonSnapshot>>,
    pub stop_flag: Arc<AtomicBool>,
}

impl DaemonHandle {
    pub fn send_command(&self, cmd: ControlCommand) -> Result<IpcResponse, String> {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx
            .send((cmd, tx))
            .map_err(|e| format!("daemon channel closed: {e}"))?;
        let raw = rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "daemon command timeout".to_string())?;
        serde_json::from_str::<IpcResponse>(&raw)
            .map_err(|e| format!("failed to parse daemon response: {e}"))
    }
}

pub fn start_daemon(enable_ipc: bool, enable_http_bridge: bool) -> DaemonHandle {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let snapshot = Arc::new(Mutex::new(DaemonSnapshot {
        awake: false,
        paused: false,
        reason: None,
        keep_display_on: false,
        remaining_seconds: None,
        battery: battery_status().map(BatteryInfo::from),
        detected_agents: Vec::new(),
        active_schedule: None,
        recent_audit: Vec::new(),
    }));

    let (cmd_tx, cmd_rx): (
        Sender<(ControlCommand, Sender<String>)>,
        Receiver<(ControlCommand, Sender<String>)>,
    ) = mpsc::channel();

    if enable_ipc {
        let _ = spawn_ipc_server(cmd_tx.clone());
    }
    if enable_http_bridge {
        if let Ok(cfg) = Config::load_or_default() {
            let _ = spawn_http_bridge(cfg.bot, cmd_tx.clone());
        }
    }

    let stop_clone = Arc::clone(&stop_flag);
    let snap_clone = Arc::clone(&snapshot);

    thread::spawn(move || {
        run_daemon_loop(stop_clone, snap_clone, cmd_rx);
    });

    DaemonHandle {
        cmd_tx,
        snapshot,
        stop_flag,
    }
}

fn push_audit(audit: &mut VecDeque<AuditEntry>, action: &str, reason: &str) {
    audit.push_front(AuditEntry {
        timestamp: Local::now().format("%H:%M:%S").to_string(),
        action: action.to_string(),
        reason: reason.to_string(),
    });
    while audit.len() > 50 {
        audit.pop_back();
    }
}

fn run_daemon_loop(
    stop_flag: Arc<AtomicBool>,
    shared_snapshot: Arc<Mutex<DaemonSnapshot>>,
    cmd_rx: Receiver<(ControlCommand, Sender<String>)>,
) {
    let mut cfg = Config::load_or_default().unwrap_or_default();
    let mut watcher = AgentWatcher::new();
    let mut sched_state = ScheduleState::default();
    let mut manual: Option<ManualSession> = None;
    let mut paused = false;
    let mut guard: Option<PowerGuard> = None;
    let mut active_key: Option<(String, bool)> = None;
    let mut audit: VecDeque<AuditEntry> = VecDeque::new();
    let mut last_habit_prompt: Option<Instant> = None;
    let mut last_config_reload = Instant::now();

    while !stop_flag.load(Ordering::Relaxed) {
        if last_config_reload.elapsed() >= Duration::from_secs(10) {
            if let Ok(fresh) = Config::load_or_default() {
                cfg = fresh;
            }
            last_config_reload = Instant::now();
        }

        // Drain any already-queued commands immediately.
        while let Ok((cmd, reply_tx)) = cmd_rx.try_recv() {
            handle_daemon_command(
                cmd,
                reply_tx,
                &cfg,
                &mut sched_state,
                &mut manual,
                &mut paused,
                &mut audit,
                &shared_snapshot,
            );
        }

        // Check manual session expiry & pre-expiry warning
        if let Some(sess) = manual.as_mut() {
            if let Some(ends) = sess.ends_at {
                let now_inst = Instant::now();
                if now_inst >= ends {
                    manual = None;
                    push_audit(&mut audit, "ended", "Manual timer finished");
                    if cfg.bot.notify_on_end {
                        send_bot_event_async(
                            &cfg.bot,
                            BotEventPayload {
                                event: "session_ended".into(),
                                title: "wakey finished".into(),
                                message: "Manual timer finished; PC can sleep normally.".into(),
                                actions: vec!["awake 1h".into()],
                                timestamp: Local::now(),
                            },
                        );
                    }
                } else {
                    let rem = ends - now_inst;
                    let warn_dur =
                        Duration::from_secs(u64::from(cfg.general.warn_before_minutes) * 60);
                    if cfg.general.warn_before_minutes > 0
                        && !sess.warned_pre_expiry
                        && rem <= warn_dur
                    {
                        sess.warned_pre_expiry = true;
                        let mins_left = (rem.as_secs() / 60).max(1);
                        let title = format!("wakey ending in {mins_left}m");
                        let body = "Do you want to extend? (Use Tray, CLI `wakey extend 30m`, or reply to Grok bot)";
                        send_desktop_notification(&title, body);
                        if cfg.bot.notify_on_pre_expiry {
                            send_bot_event_async(
                                &cfg.bot,
                                BotEventPayload {
                                    event: "pre_expiry_warning".into(),
                                    title,
                                    message: body.into(),
                                    actions: vec![
                                        "extend 15m".into(),
                                        "extend 30m".into(),
                                        "extend 1h".into(),
                                        "stop".into(),
                                    ],
                                    timestamp: Local::now(),
                                },
                            );
                        }
                    }
                }
            }
        }

        let batt = battery_status();
        let battery_blocked = if let Some(b) = batt {
            let below_min = b
                .percent
                .map(|p| !b.on_ac_power && p <= cfg.general.min_battery_percent)
                .unwrap_or(false);
            below_min || (cfg.general.pause_on_battery && !b.on_ac_power && manual.is_none())
        } else {
            false
        };

        let detected_agents = watcher.scan(&cfg.agent_watch);
        let sched_match = evaluate_schedules(&cfg.schedules, &sched_state, Local::now());

        let mut desired: Option<(String, bool, Option<u64>)> = None;

        if !paused && !battery_blocked {
            if let Some(sess) = manual.as_ref() {
                let rem = sess
                    .ends_at
                    .map(|e| e.saturating_duration_since(Instant::now()).as_secs());
                desired = Some(("Manual session".into(), sess.keep_display_on, rem));
            } else if let Some(ref s) = sched_match {
                let allowed = s.on_battery || batt.map(|b| b.on_ac_power).unwrap_or(true);
                if allowed {
                    desired = Some((
                        format!("Schedule: {}", s.name),
                        s.keep_display_on,
                        Some(s.remaining.as_secs()),
                    ));
                }
            } else if let Some(agent_reason) = watcher.evaluate(&cfg.agent_watch) {
                let allowed =
                    cfg.agent_watch.on_battery || batt.map(|b| b.on_ac_power).unwrap_or(true);
                if allowed {
                    desired = Some((agent_reason, cfg.agent_watch.keep_display_on, None));
                }
            }
        }

        // Habit prompt check when idle
        if desired.is_none() && !paused && cfg.habits.enabled {
            let cooldown_ok = last_habit_prompt
                .map(|t| {
                    t.elapsed()
                        >= Duration::from_secs(u64::from(cfg.habits.prompt_cooldown_minutes) * 60)
                })
                .unwrap_or(true);
            if cooldown_ok {
                let records = load_records(&default_history_path());
                if let Some(habit) = matching_habit_now(&records, &cfg.habits, Local::now()) {
                    last_habit_prompt = Some(Instant::now());
                    let title = format!("Keep PC awake until {}?", habit.suggested_until);
                    send_desktop_notification(&title, &habit.description);
                    if cfg.bot.notify_on_habit {
                        send_bot_event_async(
                            &cfg.bot,
                            BotEventPayload {
                                event: "habit_prompt".into(),
                                title,
                                message: habit.description,
                                actions: vec![
                                    format!("until {}", habit.suggested_until),
                                    "extend 1h".into(),
                                ],
                                timestamp: Local::now(),
                            },
                        );
                    }
                }
            }
        }

        // Apply or release OS power assertion
        match desired {
            Some((ref reason, keep_display, rem_secs)) => {
                let new_key = (reason.clone(), keep_display);
                if active_key.as_ref() != Some(&new_key) {
                    drop(guard.take());
                    let opts = PowerOptions {
                        prevent_system_sleep: true,
                        prevent_display_sleep: keep_display,
                        allow_away_mode: false,
                    };
                    if let Ok(g) = PowerGuard::acquire(opts, reason) {
                        guard = Some(g);
                        active_key = Some(new_key);
                        push_audit(&mut audit, "awake", reason);
                        if reason.starts_with("Agent active:") && cfg.bot.notify_on_agent {
                            send_bot_event_async(
                                &cfg.bot,
                                BotEventPayload {
                                    event: "agent_detected".into(),
                                    title: "wakey: AI agent detected".into(),
                                    message: reason.clone(),
                                    actions: vec!["status".into(), "pause".into()],
                                    timestamp: Local::now(),
                                },
                            );
                        }
                    }
                }

                let mut snap = shared_snapshot.lock().unwrap();
                snap.awake = true;
                snap.paused = paused;
                snap.reason = Some(reason.clone());
                snap.keep_display_on = keep_display;
                snap.remaining_seconds = rem_secs;
                snap.battery = batt.map(BatteryInfo::from);
                snap.detected_agents = detected_agents;
                snap.active_schedule = sched_match.map(|s| s.name);
                snap.recent_audit = audit.iter().cloned().collect();
            }
            None => {
                if guard.is_some() {
                    drop(guard.take());
                    active_key = None;
                    push_audit(&mut audit, "sleep_allowed", "No active wakey trigger");
                }
                let mut snap = shared_snapshot.lock().unwrap();
                snap.awake = false;
                snap.paused = paused;
                snap.reason = None;
                snap.keep_display_on = false;
                snap.remaining_seconds = None;
                snap.battery = batt.map(BatteryInfo::from);
                snap.detected_agents = detected_agents;
                snap.active_schedule = sched_match.map(|s| s.name);
                snap.recent_audit = audit.iter().cloned().collect();
            }
        }

        // Wake immediately on incoming commands, otherwise tick every 2 seconds.
        if let Ok((cmd, reply_tx)) = cmd_rx.recv_timeout(Duration::from_secs(2)) {
            handle_daemon_command(
                cmd,
                reply_tx,
                &cfg,
                &mut sched_state,
                &mut manual,
                &mut paused,
                &mut audit,
                &shared_snapshot,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_daemon_command(
    cmd: ControlCommand,
    reply_tx: Sender<String>,
    cfg: &Config,
    sched_state: &mut ScheduleState,
    manual: &mut Option<ManualSession>,
    paused: &mut bool,
    audit: &mut VecDeque<AuditEntry>,
    shared_snapshot: &Arc<Mutex<DaemonSnapshot>>,
) {
    let mut ok = true;
    let message = match cmd {
        ControlCommand::Status => "status".into(),
        ControlCommand::Pause => {
            *paused = true;
            push_audit(audit, "paused", "User paused wakey");
            "wakey paused".into()
        }
        ControlCommand::Resume => {
            *paused = false;
            push_audit(audit, "resumed", "User resumed wakey");
            "wakey resumed".into()
        }
        ControlCommand::Stop => {
            *manual = None;
            *paused = false;
            push_audit(audit, "stopped", "Manual session stopped");
            "manual wakeup stopped".into()
        }
        ControlCommand::AwakeFor { duration, display } => match parse_duration(&duration) {
            Ok(dur) => {
                let mins = (dur.as_secs() / 60).max(1) as u32;
                *manual = Some(ManualSession {
                    ends_at: Some(Instant::now() + dur),
                    total_duration_mins: mins,
                    keep_display_on: display,
                    warned_pre_expiry: false,
                });
                *paused = false;
                record_manual_session(mins, display);
                let msg = format!("Awake for {}", humantime::format_duration(dur));
                push_audit(audit, "started", &msg);
                if cfg.bot.notify_on_start {
                    send_bot_event_async(
                        &cfg.bot,
                        BotEventPayload {
                            event: "session_started".into(),
                            title: "wakey started".into(),
                            message: msg.clone(),
                            actions: vec!["extend 30m".into(), "stop".into()],
                            timestamp: Local::now(),
                        },
                    );
                }
                msg
            }
            Err(e) => {
                ok = false;
                e.to_string()
            }
        },
        ControlCommand::AwakeUntil { clock, display } => match duration_until_clock(&clock) {
            Ok(dur) => {
                let mins = (dur.as_secs() / 60).max(1) as u32;
                *manual = Some(ManualSession {
                    ends_at: Some(Instant::now() + dur),
                    total_duration_mins: mins,
                    keep_display_on: display,
                    warned_pre_expiry: false,
                });
                *paused = false;
                record_manual_session(mins, display);
                let msg = format!("Awake until {clock}");
                push_audit(audit, "started", &msg);
                msg
            }
            Err(e) => {
                ok = false;
                e.to_string()
            }
        },
        ControlCommand::Extend { duration } => match parse_duration(&duration) {
            Ok(dur) => {
                let extra_mins = (dur.as_secs() / 60).max(1) as u32;
                match manual.as_mut() {
                    Some(sess) => {
                        let base = sess.ends_at.unwrap_or_else(Instant::now);
                        sess.ends_at = Some(base.max(Instant::now()) + dur);
                        sess.total_duration_mins += extra_mins;
                        sess.warned_pre_expiry = false;
                    }
                    None => {
                        *manual = Some(ManualSession {
                            ends_at: Some(Instant::now() + dur),
                            total_duration_mins: extra_mins,
                            keep_display_on: !cfg.general.allow_screen_off_default,
                            warned_pre_expiry: false,
                        });
                    }
                }
                let msg = format!("Extended wakeup by {}", humantime::format_duration(dur));
                push_audit(audit, "extended", &msg);
                if cfg.bot.notify_on_start {
                    send_bot_event_async(
                        &cfg.bot,
                        BotEventPayload {
                            event: "session_extended".into(),
                            title: "wakey extended".into(),
                            message: msg.clone(),
                            actions: vec!["status".into(), "stop".into()],
                            timestamp: Local::now(),
                        },
                    );
                }
                msg
            }
            Err(e) => {
                ok = false;
                e.to_string()
            }
        },
        ControlCommand::SkipSchedule { name } => {
            sched_state.skip_today(&name);
            let msg = format!("Skipped schedule '{name}' for today");
            push_audit(audit, "skip_schedule", &msg);
            msg
        }
    };

    let snap_now = shared_snapshot.lock().unwrap().clone();
    let resp = IpcResponse {
        ok,
        message,
        snapshot: Some(snap_now),
    };
    let _ = reply_tx.send(serde_json::to_string(&resp).unwrap_or_default());
}

fn record_manual_session(duration_minutes: u32, keep_display: bool) {
    let now = Local::now();
    let rec = SessionRecord {
        timestamp: now,
        weekday: WeekdayShort::from_chrono(now.weekday()),
        start_minute_of_day: (now.hour() * 60 + now.minute()) as u16,
        duration_minutes,
        keep_display,
        source: "manual".into(),
    };
    let _ = append_record(&default_history_path(), &rec);
}
