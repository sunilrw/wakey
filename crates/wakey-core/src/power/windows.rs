use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerClearRequest, PowerCreateRequest, PowerRequestAwayModeRequired,
    PowerRequestDisplayRequired, PowerRequestSystemRequired, PowerSetRequest,
    POWER_REQUEST_TYPE, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT};

use super::{BatteryStatus, PowerError, PowerOptions};

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
    let on_ac_power = status.ACLineStatus == 1;
    let percent = if status.BatteryLifePercent <= 100 {
        Some(status.BatteryLifePercent)
    } else {
        None
    };
    Some(BatteryStatus {
        on_ac_power,
        percent,
    })
}
