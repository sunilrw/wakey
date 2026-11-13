use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerClearRequest, PowerCreateRequest, PowerRequestAwayModeRequired,
    PowerRequestDisplayRequired, PowerRequestSystemRequired, PowerSetRequest,
    POWER_REQUEST_TYPE, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT};

use super::{
    build_windows_powershell_script, format_windows_days_of_week, parse_clock_hhmm,
    BatteryStatus, PowerError, PowerOptions, WINDOWS_WAKE_TASK_NAME,
};
use crate::config::WeekdayShort;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct WindowsPowerRequest {
    handle: HANDLE,
    active_types: Vec<POWER_REQUEST_TYPE>,
}

// HANDLE is an opaque kernel handle safe to release from the owning/dropping thread.
unsafe impl Send for WindowsPowerRequest {}

impl WindowsPowerRequest {
    pub fn acquire(options: PowerOptions, reason: &str) -> Result<Self, PowerError> {
        let wide_reason = HSTRING::from(reason);
        let mut context = REASON_CONTEXT {
            Version: 0, // POWER_REQUEST_CONTEXT_VERSION
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            ..Default::default()
        };
        context.Reason.SimpleReasonString = windows::core::PWSTR(wide_reason.as_ptr() as *mut _);

        let handle = unsafe { PowerCreateRequest(&context) }
            .map_err(|e| PowerError::AssertionFailed(format!("PowerCreateRequest: {e}")))?;

        let mut active_types = Vec::new();
        let mut requested = Vec::new();
        if options.prevent_system_sleep {
            requested.push(PowerRequestSystemRequired);
        }
        if options.prevent_display_sleep {
            requested.push(PowerRequestDisplayRequired);
        }
        if options.allow_away_mode {
            requested.push(PowerRequestAwayModeRequired);
        }

        for req_type in requested {
            if let Err(e) = unsafe { PowerSetRequest(handle, req_type) } {
                for applied in active_types.drain(..) {
                    unsafe {
                        let _ = PowerClearRequest(handle, applied);
                    }
                }
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(PowerError::AssertionFailed(format!("PowerSetRequest: {e}")));
            }
            active_types.push(req_type);
        }

        Ok(Self {
            handle,
            active_types,
        })
    }
}

impl Drop for WindowsPowerRequest {
    fn drop(&mut self) {
        for req_type in self.active_types.drain(..) {
            unsafe {
                let _ = PowerClearRequest(self.handle, req_type);
            }
        }
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn battery_status() -> Option<BatteryStatus> {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status).ok()? };
    // ACLineStatus: 0 = Offline (on battery), 1 = Online (AC), 255 = Unknown.
    // BatteryFlag & 128 = No system battery. Only treat explicit 0 with a battery as on battery.
    let no_battery = (status.BatteryFlag & 128) != 0;
    let on_ac_power = no_battery || status.ACLineStatus != 0;
    let percent = if !no_battery && status.BatteryLifePercent <= 100 {
        Some(status.BatteryLifePercent)
    } else {
        None
    };
    Some(BatteryStatus {
        on_ac_power,
        percent,
    })
}

pub fn schedule_morning_wake(days: &[WeekdayShort], clock_hhmm: &str) -> Result<String, String> {
    let script = build_windows_powershell_script(days, clock_hhmm)?;
    let days_label = format_windows_days_of_week(days)?;
    let (h, m) = parse_clock_hhmm(clock_hhmm)?;

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to launch powershell.exe: {e}"))?;

    if output.status.success() {
        Ok(format!(
            "Scheduled Windows Task '{WINDOWS_WAKE_TASK_NAME}' with WakeToRun ({days_label} at {h:02}:{m:02})."
        ))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        Err(format!(
            "Failed to create Windows wake task '{WINDOWS_WAKE_TASK_NAME}': {detail}"
        ))
    }
}

pub fn clear_morning_wake() -> Result<String, String> {
    let output = Command::new("schtasks")
        .args(["/Delete", "/TN", WINDOWS_WAKE_TASK_NAME, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to launch schtasks: {e}"))?;

    if output.status.success() {
        return Ok(format!(
            "Deleted Windows Task '{WINDOWS_WAKE_TASK_NAME}'."
        ));
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("cannot find") || lower.contains("does not exist") {
        Ok(format!(
            "No Windows wake task '{WINDOWS_WAKE_TASK_NAME}' was scheduled."
        ))
    } else {
        Err(format!(
            "Failed to delete Windows wake task '{WINDOWS_WAKE_TASK_NAME}': {stderr}"
        ))
    }
}
