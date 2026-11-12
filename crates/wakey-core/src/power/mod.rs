use thiserror::Error;

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
