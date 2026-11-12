use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use wakey_core::agent_watch::AgentWatcher;
use wakey_core::bot_bridge::ControlCommand;
use wakey_core::config::{Config, ScheduleRule, WeekdayShort};
use wakey_core::daemon::start_daemon;
use wakey_core::habits::{default_history_path, detect_habits, load_records};
use wakey_core::ipc::send_ipc_command;
use wakey_core::power::{battery_status, PowerGuard, PowerOptions};
use wakey_core::process::{run_and_wait, wait_for_pid};
use wakey_core::time_parse::{duration_until_clock, parse_duration};

#[derive(Parser, Debug)]
#[command(
    name = "wakey",
    version,
    about = "Cross-platform caffeinate parity with smart schedules, AI agent detection, and habit learning"
)]
struct Cli {
    #[command(subcommand)]
    subcommand: Option<Commands>,

    /// Prevent the display from turning off (also prevents idle screen lock)
    #[arg(short = 'd', long = "display")]
    display: bool,

    /// Keep the system awake while allowing the display to turn off (default)
    #[arg(short = 's', long = "system")]
    system: bool,

    /// Allow Away Mode on Windows (useful for media/background tasks)
    #[arg(short = 'a', long = "away")]
    away: bool,

    /// Stay awake for a duration (e.g. '30m', '2h', '1h30m', or seconds like '300')
    #[arg(short = 't', long = "time", value_name = "DURATION")]
    time: Option<String>,

    /// Stay awake until a local 24-hour clock time (e.g. '18:00' or '23:30')
    #[arg(long = "until", value_name = "HH:MM", conflicts_with = "time")]
    until: Option<String>,

    /// Stay awake while the process with this PID is running
    #[arg(short = 'w', long = "wait", value_name = "PID")]
    wait_pid: Option<u32>,

    /// Human-readable reason shown in OS power diagnostics
    #[arg(short = 'r', long = "reason", default_value = "wakey CLI session")]
    reason: String,

    /// Command to run while keeping the machine awake (use after `--`)
    #[arg(last = true, value_name = "COMMAND")]
    command: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the background daemon (schedules, agent watch, habit prompts, IPC & Grok bot HTTP bridge)
    Daemon,
    /// Show current wakey status (queries running daemon, or local state if daemon is stopped)
    Status,
    /// Stop active manual wakeup in the running daemon
    Stop,
    /// Pause all wakey triggers in the running daemon
    Pause,
    /// Resume wakey triggers in the running daemon
    Resume,
    /// Extend the active wakeup (e.g. `wakey extend 30m`)
    Extend {
        /// Extra duration (e.g. 15m, 30m, 1h)
        duration: String,
    },
    /// Manage daily & weekly schedules
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },
    /// Scan and display currently detected AI agents / watched processes
    Agents,
    /// Show learned usage habits
    Habits,
    /// Run diagnostics on OS power assertions, battery, and config
    Doctor,
}

#[derive(Subcommand, Debug)]
enum ScheduleAction {
    List,
    Add {
        #[arg(long)]
        name: String,
        /// Days: 'daily', 'mon-fri', 'weekends', or comma-separated 'mon,wed,fri'
        #[arg(long)]
        days: String,
        #[arg(long)]
        start: String,
        #[arg(long)]
        end: String,
        #[arg(long)]
        display: bool,
        #[arg(long)]
        on_battery: bool,
    },
    Remove {
        name: String,
    },
    SkipToday {
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(sub) = cli.subcommand {
        return handle_subcommand(sub);
    }

    let options = PowerOptions {
        prevent_system_sleep: true,
        prevent_display_sleep: cli.display,
        allow_away_mode: cli.away,
    };

    let reason = if !cli.command.is_empty() {
        format!("wakey running: {}", cli.command.join(" "))
    } else if let Some(pid) = cli.wait_pid {
        format!("wakey watching PID {pid}")
    } else {
        cli.reason.clone()
    };

    let stop_flag = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop_flag);
        ctrlc::set_handler(move || {
            stop.store(true, Ordering::SeqCst);
        })
        .context("failed to install Ctrl+C handler")?;
    }

    let _guard = PowerGuard::acquire(options, &reason)
        .context("could not acquire OS power assertion")?;

    let mode_label = if options.prevent_display_sleep {
        "system + display awake"
    } else {
        "system awake (display allowed to turn off)"
    };

    if let Some(batt) = battery_status() {
        let power_src = if batt.on_ac_power { "AC" } else { "Battery" };
        let pct = batt
            .percent
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "n/a".into());
        eprintln!("wakey: {mode_label} [{power_src} {pct}]");
    } else {
        eprintln!("wakey: {mode_label}");
    }

    if let Some((program, args)) = cli.command.split_first() {
        let code = run_and_wait(program, args)?;
        std::process::exit(code);
    }

    if let Some(pid) = cli.wait_pid {
        eprintln!("wakey: waiting for PID {pid} to exit (Ctrl+C to stop)...");
        wait_for_pid(pid, &stop_flag)?;
        eprintln!("wakey: released power assertion.");
        return Ok(());
    }

    let target_duration = if let Some(ref dur_str) = cli.time {
        Some(parse_duration(dur_str)?)
    } else if let Some(ref until_str) = cli.until {
        Some(duration_until_clock(until_str)?)
    } else {
        None
    };

    if let Some(dur) = target_duration {
        if dur.is_zero() {
            bail!("duration must be greater than 0");
        }
        eprintln!(
            "wakey: active for {} (Ctrl+C to stop early)...",
            humantime::format_duration(dur)
        );
        let start = Instant::now();
        while !stop_flag.load(Ordering::Relaxed) && start.elapsed() < dur {
            let remaining = dur.saturating_sub(start.elapsed());
            thread::sleep(remaining.min(Duration::from_millis(250)));
        }
        eprintln!("wakey: released power assertion.");
        return Ok(());
    }

    eprintln!("wakey: active indefinitely (press Ctrl+C to stop)...");
    while !stop_flag.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(250));
    }
    eprintln!("wakey: released power assertion.");
    Ok(())
}

fn handle_subcommand(sub: Commands) -> Result<()> {
    match sub {
        Commands::Daemon => {
            let handle = start_daemon(true, true);
            eprintln!("wakey daemon running (Ctrl+C to stop)...");
            let stop_wait = Arc::new(AtomicBool::new(false));
            let s = Arc::clone(&stop_wait);
            let daemon_stop = Arc::clone(&handle.stop_flag);
            ctrlc::set_handler(move || {
                daemon_stop.store(true, Ordering::SeqCst);
                s.store(true, Ordering::SeqCst);
            })?;
            while !stop_wait.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(250));
            }
            eprintln!("wakey daemon stopped.");
            Ok(())
        }
        Commands::Status => {
            match send_ipc_command(&ControlCommand::Status) {
                Ok(resp) => {
                    if let Some(snap) = resp.snapshot {
                        println!(
                            "Daemon: running | Awake: {} | Paused: {}",
                            snap.awake, snap.paused
                        );
                        if let Some(r) = snap.reason {
                            println!("Reason: {r}");
                        }
                        if let Some(rem) = snap.remaining_seconds {
                            println!("Remaining: {}s", rem);
                        }
                        println!("Detected agents: {}", snap.detected_agents.len());
                    } else {
                        println!("{}", resp.message);
                    }
                }
                Err(_) => {
                    println!("Daemon: not running");
                    if let Some(b) = battery_status() {
                        println!(
                            "Power: {} ({}%)",
                            if b.on_ac_power { "AC" } else { "Battery" },
                            b.percent.map(|p| p.to_string()).unwrap_or("?".into())
                        );
                    }
                }
            }
            Ok(())
        }
        Commands::Stop => {
            let resp = send_ipc_command(&ControlCommand::Stop).map_err(anyhow::Error::msg)?;
            println!("{}", resp.message);
            Ok(())
        }
        Commands::Pause => {
            let resp = send_ipc_command(&ControlCommand::Pause).map_err(anyhow::Error::msg)?;
            println!("{}", resp.message);
            Ok(())
        }
        Commands::Resume => {
            let resp = send_ipc_command(&ControlCommand::Resume).map_err(anyhow::Error::msg)?;
            println!("{}", resp.message);
            Ok(())
        }
        Commands::Extend { duration } => {
            let resp = send_ipc_command(&ControlCommand::Extend { duration })
                .map_err(anyhow::Error::msg)?;
            println!("{}", resp.message);
            Ok(())
        }
        Commands::Schedule { action } => handle_schedule(action),
        Commands::Agents => {
            let cfg = Config::load_or_default()?;
            let mut watcher = AgentWatcher::new();
            // Brief sleep so sysinfo can compute non-zero delta CPU usage
            thread::sleep(Duration::from_millis(300));
            let found = watcher.scan(&cfg.agent_watch);
            if found.is_empty() {
                println!("No watched AI agents currently running.");
            } else {
                for a in found {
                    println!(
                        "- {} (pid {}, proc '{}', cpu {:.1}%, children {}, active={})",
                        a.name,
                        a.pid,
                        a.process_name,
                        a.cpu_percent,
                        a.active_children,
                        a.is_actively_working
                    );
                }
            }
            Ok(())
        }
        Commands::Habits => {
            let cfg = Config::load_or_default()?;
            let records = load_records(&default_history_path());
            let habits = detect_habits(&records, &cfg.habits);
            if habits.is_empty() {
                println!(
                    "No learned habits yet ({} recorded manual sessions; min_occurrences={}).",
                    records.len(),
                    cfg.habits.min_occurrences
                );
            } else {
                for h in habits {
                    println!("- [{}x] {}", h.occurrences, h.description);
                }
            }
            Ok(())
        }
        Commands::Doctor => {
            println!("OS: {}", std::env::consts::OS);
            println!("Config path: {}", Config::default_path().display());
            println!("Data dir: {}", Config::data_dir().display());
            match battery_status() {
                Some(b) => println!(
                    "Battery: {} ({}%)",
                    if b.on_ac_power { "AC Power" } else { "On Battery" },
                    b.percent.map(|p| p.to_string()).unwrap_or("n/a".into())
                ),
                None => println!("Battery: unavailable / desktop"),
            }
            match PowerGuard::acquire(PowerOptions::default(), "wakey doctor check") {
                Ok(_g) => println!("Power assertion test: OK (acquired & released cleanly)"),
                Err(e) => println!("Power assertion test: FAILED ({e})"),
            }
            #[cfg(target_os = "windows")]
            println!("Windows tip: Check `powercfg /requests` while active. Note that closing a laptop lid on Modern Standby may still sleep depending on your Windows Power Plan.");
            #[cfg(target_os = "macos")]
            println!("macOS tip: Check `pmset -g assertions` while active to inspect wakey's IOKit assertion.");
            Ok(())
        }
    }
}

fn handle_schedule(action: ScheduleAction) -> Result<()> {
    let mut cfg = Config::load_or_default()?;
    match action {
        ScheduleAction::List => {
            if cfg.schedules.is_empty() {
                println!("No schedules configured.");
            } else {
                for s in &cfg.schedules {
                    println!(
                        "- {} [{:?}] {}-{} (display={}, battery={}, enabled={})",
                        s.name, s.days, s.start, s.end, s.keep_display_on, s.on_battery, s.enabled
                    );
                }
            }
        }
        ScheduleAction::Add {
            name,
            days,
            start,
            end,
            display,
            on_battery,
        } => {
            let parsed_days = WeekdayShort::parse_list(&days);
            if parsed_days.is_empty() {
                bail!("could not parse days '{days}' (try 'mon-fri', 'daily', or 'sat,sun')");
            }
            cfg.schedules.retain(|s| s.name != name);
            cfg.schedules.push(ScheduleRule {
                name: name.clone(),
                days: parsed_days,
                start,
                end,
                keep_display_on: display,
                on_battery,
                enabled: true,
            });
            cfg.save()?;
            println!("Saved schedule '{name}'.");
        }
        ScheduleAction::Remove { name } => {
            let before = cfg.schedules.len();
            cfg.schedules.retain(|s| s.name != name);
            cfg.save()?;
            if cfg.schedules.len() < before {
                println!("Removed schedule '{name}'.");
            } else {
                println!("Schedule '{name}' not found.");
            }
        }
        ScheduleAction::SkipToday { name } => {
            let resp = send_ipc_command(&ControlCommand::SkipSchedule { name })
                .map_err(anyhow::Error::msg)?;
            println!("{}", resp.message);
        }
    }
    Ok(())
}
