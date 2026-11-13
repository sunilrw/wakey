use chrono::{Datelike, Duration, Local, TimeZone};
use thiserror::Error;

use crate::config::WeekdayShort;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod linux;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerOptions {
    /// Prevent the system from idling to sleep.
    pub prevent_system_sleep: bool,
    /// Prevent the display from turning off (also prevents idle screen lock).
    pub prevent_display_sleep: bool,
    /// Allow Away Mode on Windows (useful for background/media tasks).
    pub allow_away_mode: bool,
}

impl Default for PowerOptions {
    fn default() -> Self {
        Self {
            prevent_system_sleep: true,
            prevent_display_sleep: false,
            allow_away_mode: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    pub on_ac_power: bool,
    pub percent: Option<u8>,
}

#[derive(Debug, Error)]
pub enum PowerError {
    #[error("failed to acquire OS power assertion: {0}")]
    AssertionFailed(String),
    #[error("unsupported platform")]
    UnsupportedPlatform,
}

/// RAII guard that keeps the machine awake until dropped.
#[allow(dead_code)]
pub struct PowerGuard {
    #[cfg(target_os = "macos")]
    inner: macos::MacOsAssertion,
    #[cfg(target_os = "windows")]
    inner: windows::WindowsPowerRequest,
    #[cfg(target_os = "linux")]
    inner: linux::LinuxInhibitor,
}

impl PowerGuard {
    pub fn acquire(options: PowerOptions, reason: &str) -> Result<Self, PowerError> {
        #[cfg(target_os = "macos")]
        {
            return Ok(Self {
                inner: macos::MacOsAssertion::acquire(options, reason)?,
            });
        }
        #[cfg(target_os = "windows")]
        {
            return Ok(Self {
                inner: windows::WindowsPowerRequest::acquire(options, reason)?,
            });
        }
        #[cfg(target_os = "linux")]
        {
            return Ok(Self {
                inner: linux::LinuxInhibitor::acquire(options, reason)?,
            });
        }
        #[allow(unreachable_code)]
        Err(PowerError::UnsupportedPlatform)
    }
}

/// Queries the current AC/battery power status if available on this OS.
pub fn battery_status() -> Option<BatteryStatus> {
    #[cfg(target_os = "windows")]
    {
        return windows::battery_status();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::battery_status();
    }
    #[cfg(target_os = "linux")]
    {
        return linux::battery_status();
    }
    #[allow(unreachable_code)]
    None
}

#[allow(dead_code)]
pub const WINDOWS_WAKE_TASK_NAME: &str = "wakey-morning-wake";

/// Validates and parses an `HH:MM` 24-hour clock string.
pub(crate) fn parse_clock_hhmm(clock_hhmm: &str) -> Result<(u32, u32), String> {
    let trimmed = clock_hhmm.trim();
    let mut parts = trimmed.split(':');
    let h_str = parts.next().unwrap_or("");
    let m_str = parts.next().unwrap_or("");
    if parts.next().is_some()
        || h_str.len() != 2
        || m_str.len() != 2
        || !h_str.bytes().all(|b| b.is_ascii_digit())
        || !m_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(format!(
            "Invalid wake time '{clock_hhmm}': expected HH:MM (00:00–23:59)"
        ));
    }
    let hour: u32 = h_str
        .parse()
        .map_err(|_| format!("Invalid hour in '{clock_hhmm}'"))?;
    let minute: u32 = m_str
        .parse()
        .map_err(|_| format!("Invalid minute in '{clock_hhmm}'"))?;
    if hour > 23 || minute > 59 {
        return Err(format!(
            "Invalid wake time '{clock_hhmm}': hour must be 00–23 and minute 00–59"
        ));
    }
    Ok((hour, minute))
}

/// Deduplicates weekdays in canonical Mon–Sun order.
fn canonical_days(days: &[WeekdayShort]) -> Result<Vec<WeekdayShort>, String> {
    if days.is_empty() {
        return Err("Select at least one weekday for morning wake.".to_string());
    }
    let order = [
        WeekdayShort::Mon,
        WeekdayShort::Tue,
        WeekdayShort::Wed,
        WeekdayShort::Thu,
        WeekdayShort::Fri,
        WeekdayShort::Sat,
        WeekdayShort::Sun,
    ];
    let out: Vec<WeekdayShort> = order
        .into_iter()
        .filter(|d| days.contains(d))
        .collect();
    if out.is_empty() {
        return Err("Select at least one weekday for morning wake.".to_string());
    }
    Ok(out)
}

/// Formats weekdays into macOS `pmset repeat` day codes (`M T W R F S U`).
#[allow(dead_code)]
pub(crate) fn format_macos_pmset_days(days: &[WeekdayShort]) -> Result<String, String> {
    let deduped = canonical_days(days)?;
    let mut s = String::with_capacity(deduped.len());
    for d in deduped {
        s.push(match d {
            WeekdayShort::Mon => 'M',
            WeekdayShort::Tue => 'T',
            WeekdayShort::Wed => 'W',
            WeekdayShort::Thu => 'R',
            WeekdayShort::Fri => 'F',
            WeekdayShort::Sat => 'S',
            WeekdayShort::Sun => 'U',
        });
    }
    Ok(s)
}

/// Builds the `pmset repeat wakeorpoweron <days> <HH:MM:00>` command string for macOS.
#[allow(dead_code)]
pub(crate) fn build_macos_pmset_command(
    days: &[WeekdayShort],
    clock_hhmm: &str,
) -> Result<String, String> {
    let pmset_days = format_macos_pmset_days(days)?;
    let (hour, minute) = parse_clock_hhmm(clock_hhmm)?;
    Ok(format!(
        "pmset repeat wakeorpoweron {pmset_days} {hour:02}:{minute:02}:00"
    ))
}

/// Formats weekdays into PowerShell `DaysOfWeek` list (`Monday,Tuesday,...`).
#[allow(dead_code)]
pub(crate) fn format_windows_days_of_week(days: &[WeekdayShort]) -> Result<String, String> {
    let deduped = canonical_days(days)?;
    let names: Vec<&'static str> = deduped
        .into_iter()
        .map(|d| match d {
            WeekdayShort::Mon => "Monday",
            WeekdayShort::Tue => "Tuesday",
            WeekdayShort::Wed => "Wednesday",
            WeekdayShort::Thu => "Thursday",
            WeekdayShort::Fri => "Friday",
            WeekdayShort::Sat => "Saturday",
            WeekdayShort::Sun => "Sunday",
        })
        .collect();
    Ok(names.join(","))
}

/// Builds the PowerShell script to register a Task Scheduler task with `WakeToRun` enabled.
#[allow(dead_code)]
pub(crate) fn build_windows_powershell_script(
    days: &[WeekdayShort],
    clock_hhmm: &str,
) -> Result<String, String> {
    let days_arg = format_windows_days_of_week(days)?;
    let (hour, minute) = parse_clock_hhmm(clock_hhmm)?;
    let at_str = format!("{hour:02}:{minute:02}");
    Ok(format!(
        "$action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument '/c exit 0'; \
         $trigger = New-ScheduledTaskTrigger -Weekly -DaysOfWeek {days_arg} -At '{at_str}'; \
         $settings = New-ScheduledTaskSettingsSet -WakeToRun -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable; \
         Register-ScheduledTask -TaskName '{WINDOWS_WAKE_TASK_NAME}' -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null"
    ))
}

/// Computes the next upcoming local `DateTime` matching any of `days` at `clock_hhmm`.
#[allow(dead_code)]
pub(crate) fn next_wake_datetime(
    days: &[WeekdayShort],
    clock_hhmm: &str,
    now: chrono::DateTime<Local>,
) -> Result<chrono::DateTime<Local>, String> {
    let deduped = canonical_days(days)?;
    let (hour, minute) = parse_clock_hhmm(clock_hhmm)?;

    for offset in 0..=7_i64 {
        let date = now.date_naive() + Duration::days(offset);
        let wd = WeekdayShort::from_chrono(date.weekday());
        if !deduped.contains(&wd) {
            continue;
        }
        let Some(naive_dt) = date.and_hms_opt(hour, minute, 0) else {
            continue;
        };
        if let Some(candidate) = Local.from_local_datetime(&naive_dt).earliest() {
            if candidate > now {
                return Ok(candidate);
            }
        }
    }

    Err("Could not compute next wake time for the selected weekdays.".to_string())
}

/// Schedules an OS hardware/RTC morning wake for the given weekdays and `HH:MM` time.
pub fn schedule_morning_wake(days: &[WeekdayShort], clock_hhmm: &str) -> Result<String, String> {
    // Validate inputs uniformly across all platforms before dispatching.
    let _ = canonical_days(days)?;
    let _ = parse_clock_hhmm(clock_hhmm)?;

    #[cfg(target_os = "macos")]
    {
        return macos::schedule_morning_wake(days, clock_hhmm);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::schedule_morning_wake(days, clock_hhmm);
    }
    #[cfg(target_os = "linux")]
    {
        return linux::schedule_morning_wake(days, clock_hhmm);
    }
    #[allow(unreachable_code)]
    Err("Hardware/RTC wake scheduling is not supported on this platform.".to_string())
}

/// Clears any previously scheduled OS hardware/RTC morning wake.
pub fn clear_morning_wake() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        return macos::clear_morning_wake();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::clear_morning_wake();
    }
    #[cfg(target_os = "linux")]
    {
        return linux::clear_morning_wake();
    }
    #[allow(unreachable_code)]
    Err("Hardware/RTC wake scheduling is not supported on this platform.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_hhmm_strictly() {
        assert_eq!(parse_clock_hhmm("07:30"), Ok((7, 30)));
        assert_eq!(parse_clock_hhmm("00:00"), Ok((0, 0)));
        assert_eq!(parse_clock_hhmm("23:59"), Ok((23, 59)));
        assert!(parse_clock_hhmm("7:30").is_err());
        assert!(parse_clock_hhmm("24:00").is_err());
        assert!(parse_clock_hhmm("12:60").is_err());
        assert!(parse_clock_hhmm("12:30:00").is_err());
        assert!(parse_clock_hhmm("ab:cd").is_err());
        assert!(parse_clock_hhmm("").is_err());
    }

    #[test]
    fn formats_macos_pmset_command_and_dedups_days() {
        let days = [
            WeekdayShort::Fri,
            WeekdayShort::Mon,
            WeekdayShort::Wed,
            WeekdayShort::Mon,
            WeekdayShort::Thu,
        ];
        assert_eq!(format_macos_pmset_days(&days).unwrap(), "MWRF");
        assert_eq!(
            build_macos_pmset_command(&days, "07:15").unwrap(),
            "pmset repeat wakeorpoweron MWRF 07:15:00"
        );
        assert_eq!(
            format_macos_pmset_days(&[WeekdayShort::Sat, WeekdayShort::Sun]).unwrap(),
            "SU"
        );
        assert!(format_macos_pmset_days(&[]).is_err());
    }

    #[test]
    fn formats_windows_task_scheduler_script() {
        let days = [WeekdayShort::Mon, WeekdayShort::Tue, WeekdayShort::Sun];
        assert_eq!(
            format_windows_days_of_week(&days).unwrap(),
            "Monday,Tuesday,Sunday"
        );
        let script = build_windows_powershell_script(&days, "06:45").unwrap();
        assert!(script.contains("-WakeToRun"));
        assert!(script.contains("-TaskName 'wakey-morning-wake'"));
        assert!(script.contains("-DaysOfWeek Monday,Tuesday,Sunday"));
        assert!(script.contains("-At '06:45'"));
    }

    #[test]
    fn computes_next_wake_datetime_for_linux_rtc() {
        // 2026-11-13 is a Friday
        let now_naive = chrono::NaiveDate::from_ymd_opt(2026, 11, 13)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        let now = Local.from_local_datetime(&now_naive).unwrap();

        // Next Friday at 09:00 should be +7 days (since 09:00 today already passed)
        let next_fri = next_wake_datetime(&[WeekdayShort::Fri], "09:00", now).unwrap();
        assert_eq!(
            next_fri.date_naive(),
            chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap()
        );

        // Today at 11:00 should be today
        let later_today = next_wake_datetime(&[WeekdayShort::Fri], "11:00", now).unwrap();
        assert_eq!(
            later_today.date_naive(),
            chrono::NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()
        );

        // Saturday at 08:00 should be tomorrow
        let tomorrow = next_wake_datetime(&[WeekdayShort::Sat], "08:00", now).unwrap();
        assert_eq!(
            tomorrow.date_naive(),
            chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
        );
    }
}
