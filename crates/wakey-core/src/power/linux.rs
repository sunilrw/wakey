use std::process::{Child, Command, Stdio};

use super::{BatteryStatus, PowerError, PowerOptions};

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
    let mut on_ac = false;
    let mut percent = None;

    for entry in entries.flatten() {
        let path = entry.path();
        let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() == "Mains" {
            if std::fs::read_to_string(path.join("online"))
                .unwrap_or_default()
                .trim()
                == "1"
            {
                on_ac = true;
            }
        } else if kind.trim() == "Battery" && percent.is_none() {
            percent = std::fs::read_to_string(path.join("capacity"))
                .ok()
                .and_then(|s| s.trim().parse::<u8>().ok());
        }
    }

    Some(BatteryStatus {
        on_ac_power: on_ac,
        percent,
    })
}
