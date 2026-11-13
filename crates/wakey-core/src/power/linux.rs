use std::process::{Child, Command, Stdio};

use chrono::Local;

use super::{next_wake_datetime, BatteryStatus, PowerError, PowerOptions};
use crate::config::WeekdayShort;

pub struct LinuxInhibitor {
    child: Child,
}

impl LinuxInhibitor {
    pub fn acquire(options: PowerOptions, reason: &str) -> Result<Self, PowerError> {
        let what = if options.prevent_display_sleep {
            "idle:sleep"
        } else {
            "sleep"
        };

        let child = Command::new("systemd-inhibit")
            .args([
                &format!("--what={what}"),
                "--who=wakey",
                &format!("--why={reason}"),
                "--mode=block",
                "sleep",
                "infinity",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| PowerError::AssertionFailed(format!("systemd-inhibit: {e}")))?;

        Ok(Self { child })
    }
}

impl Drop for LinuxInhibitor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn battery_status() -> Option<BatteryStatus> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut saw_mains = false;
    let mut on_ac = false;
    let mut saw_battery = false;
    let mut percent = None;

    for entry in entries.flatten() {
        let path = entry.path();
        let scope = std::fs::read_to_string(path.join("scope")).unwrap_or_default();
        if scope.trim() == "Device" {
            continue;
        }
        let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() == "Mains" {
            saw_mains = true;
            if std::fs::read_to_string(path.join("online"))
                .unwrap_or_default()
                .trim()
                == "1"
            {
                on_ac = true;
            }
        } else if kind.trim() == "Battery" {
            saw_battery = true;
            if percent.is_none() {
                percent = std::fs::read_to_string(path.join("capacity"))
                    .ok()
                    .and_then(|s| s.trim().parse::<u8>().ok());
            }
        }
    }

    if !saw_mains && !saw_battery {
        return None;
    }

    Some(BatteryStatus {
        on_ac_power: if saw_mains { on_ac } else { true },
        percent,
    })
}

pub fn schedule_morning_wake(days: &[WeekdayShort], clock_hhmm: &str) -> Result<String, String> {
    let next_dt = next_wake_datetime(days, clock_hhmm, Local::now())?;
    let epoch = next_dt.timestamp();
    let epoch_str = epoch.to_string();

    match Command::new("rtcwake")
        .args(["-m", "no", "-t", &epoch_str])
        .output()
    {
        Ok(output) if output.status.success() => Ok(format!(
            "Scheduled next Linux RTC wake for {} (epoch {epoch}). Note: rtcwake schedules one wake event at a time.",
            next_dt.format("%a %Y-%m-%d %H:%M")
        )),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if stderr.is_empty() {
                format!("exit status {}", output.status)
            } else {
                stderr
            };
            Err(format!(
                "Could not set Linux RTC wake ({detail}). Run with root privileges: sudo rtcwake -m no -t {epoch}"
            ))
        }
        Err(e) => Err(format!(
            "rtcwake unavailable ({e}). To schedule hardware wake manually, run: sudo rtcwake -m no -t {epoch}"
        )),
    }
}

pub fn clear_morning_wake() -> Result<String, String> {
    match Command::new("rtcwake").args(["-m", "disable"]).output() {
        Ok(output) if output.status.success() => {
            Ok("Disabled Linux RTC wake alarm.".to_string())
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if stderr.is_empty() {
                format!("exit status {}", output.status)
            } else {
                stderr
            };
            Err(format!(
                "Could not disable Linux RTC wake ({detail}). Run with root privileges: sudo rtcwake -m disable"
            ))
        }
        Err(e) => Err(format!(
            "rtcwake unavailable ({e}). To clear manually, run: sudo rtcwake -m disable"
        )),
    }
}
