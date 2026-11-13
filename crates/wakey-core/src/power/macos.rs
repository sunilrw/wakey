use std::ffi::{c_char, c_void, CString};
use std::process::Command;

use super::{build_macos_pmset_command, format_macos_pmset_days, parse_clock_hhmm, BatteryStatus, PowerError, PowerOptions};
use crate::config::WeekdayShort;

type CFAllocatorRef = *const c_void;
type CFStringRef = *const c_void;
type IOPMAssertionID = u32;
type IOPMAssertionLevel = u32;
type IOReturn = i32;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_IOPM_ASSERTION_LEVEL_ON: IOPMAssertionLevel = 255;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        assertion_level: IOPMAssertionLevel,
        assertion_name: CFStringRef,
        assertion_id: *mut IOPMAssertionID,
    ) -> IOReturn;
    fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
}

fn make_cf_string(s: &str) -> Result<CFStringRef, PowerError> {
    let c_str = CString::new(s).map_err(|e| PowerError::AssertionFailed(e.to_string()))?;
    let cf = unsafe { CFStringCreateWithCString(std::ptr::null(), c_str.as_ptr(), K_CF_STRING_ENCODING_UTF8) };
    if cf.is_null() {
        return Err(PowerError::AssertionFailed(
            "CFStringCreateWithCString returned null".into(),
        ));
    }
    Ok(cf)
}

fn create_assertion(assertion_type: &str, reason: &str) -> Result<IOPMAssertionID, PowerError> {
    let cf_type = make_cf_string(assertion_type)?;
    let cf_reason = make_cf_string(reason);
    let cf_reason = match cf_reason {
        Ok(r) => r,
        Err(e) => {
            unsafe { CFRelease(cf_type) };
            return Err(e);
        }
    };

    let mut id: IOPMAssertionID = 0;
    let status = unsafe {
        IOPMAssertionCreateWithName(cf_type, K_IOPM_ASSERTION_LEVEL_ON, cf_reason, &mut id)
    };

    unsafe {
        CFRelease(cf_reason);
        CFRelease(cf_type);
    }

    if status != 0 {
        return Err(PowerError::AssertionFailed(format!(
            "IOPMAssertionCreateWithName({assertion_type}) failed with IOReturn {status:#x}"
        )));
    }
    Ok(id)
}

pub struct MacOsAssertion {
    assertion_ids: Vec<IOPMAssertionID>,
}

impl MacOsAssertion {
    pub fn acquire(options: PowerOptions, reason: &str) -> Result<Self, PowerError> {
        let mut assertion_ids = Vec::new();

        if options.prevent_display_sleep {
            // PreventUserIdleDisplaySleep keeps both display and system awake on macOS.
            assertion_ids.push(create_assertion("PreventUserIdleDisplaySleep", reason)?);
        } else if options.prevent_system_sleep {
            assertion_ids.push(create_assertion("PreventUserIdleSystemSleep", reason)?);
        }

        Ok(Self { assertion_ids })
    }
}

impl Drop for MacOsAssertion {
    fn drop(&mut self) {
        for id in self.assertion_ids.drain(..) {
            unsafe {
                let _ = IOPMAssertionRelease(id);
            }
        }
    }
}

pub fn battery_status() -> Option<BatteryStatus> {
    let output = Command::new("pmset").args(["-g", "batt"]).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let on_ac_power = text.contains("AC Power");
    let percent = text
        .split('%')
        .next()
        .and_then(|before_pct| before_pct.split_whitespace().last())
        .and_then(|num| num.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u8>().ok());

    Some(BatteryStatus {
        on_ac_power,
        percent,
    })
}

fn run_osascript_privileged(shell_cmd: &str) -> Result<(), String> {
    // `shell_cmd` is strictly generated from validated ASCII tokens (`pmset repeat ...`).
    let script = format!("do shell script \"{shell_cmd}\" with administrator privileges");
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("failed to launch osascript: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.contains("User canceled") || stderr.contains("-128") {
        Err("Administrator prompt was canceled.".to_string())
    } else if stderr.is_empty() {
        Err(format!("osascript exited with status {}", output.status))
    } else {
        Err(stderr)
    }
}

pub fn schedule_morning_wake(days: &[WeekdayShort], clock_hhmm: &str) -> Result<String, String> {
    let cmd = build_macos_pmset_command(days, clock_hhmm)?;
    let pmset_days = format_macos_pmset_days(days)?;
    let (h, m) = parse_clock_hhmm(clock_hhmm)?;

    match run_osascript_privileged(&cmd) {
        Ok(()) => Ok(format!(
            "Scheduled macOS hardware wake ({pmset_days} at {h:02}:{m:02})."
        )),
        Err(reason) => Err(format!(
            "{reason} To schedule manually in Terminal, run: sudo {cmd}"
        )),
    }
}

pub fn clear_morning_wake() -> Result<String, String> {
    let cmd = "pmset repeat cancel";
    match run_osascript_privileged(cmd) {
        Ok(()) => Ok("Cleared macOS hardware wake schedule.".to_string()),
        Err(reason) => Err(format!(
            "{reason} To clear manually in Terminal, run: sudo {cmd}"
        )),
    }
}
