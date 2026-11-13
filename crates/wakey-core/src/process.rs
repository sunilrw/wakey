use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to spawn command '{0}': {1}")]
    SpawnFailed(String, std::io::Error),
    #[error("process {0} could not be watched: {1}")]
    WatchFailed(u32, String),
}

/// Spawns `program` with `args` and waits for it to exit, returning its exit code.
pub fn run_and_wait(program: &str, args: &[String]) -> Result<i32, ProcessError> {
    let mut child = Command::new(program)
        .args(args)
        .spawn()
        .map_err(|e| ProcessError::SpawnFailed(program.to_string(), e))?;

    let status = child
        .wait()
        .map_err(|e| ProcessError::SpawnFailed(program.to_string(), e))?;

    Ok(status.code().unwrap_or(1))
}

/// Blocks until `pid` exits or `stop_flag` becomes `true`.
pub fn wait_for_pid(pid: u32, stop_flag: &Arc<AtomicBool>) -> Result<(), ProcessError> {
    #[cfg(windows)]
    {
        return wait_for_pid_windows(pid, stop_flag);
    }
    #[cfg(unix)]
    {
        return wait_for_pid_unix(pid, stop_flag);
    }
    #[allow(unreachable_code)]
    Err(ProcessError::WatchFailed(pid, "unsupported OS".into()))
}

#[cfg(unix)]
fn wait_for_pid_unix(pid: u32, stop_flag: &Arc<AtomicBool>) -> Result<(), ProcessError> {
    let pid_str = pid.to_string();
    while !stop_flag.load(Ordering::Relaxed) {
        let alive = Command::new("kill")
            .args(["-0", &pid_str])
            .stdout(std::process::Stdio::null())
            .output()
            .map(|out| {
                out.status.success()
                    || String::from_utf8_lossy(&out.stderr)
                        .to_lowercase()
                        .contains("permitted")
            })
            .unwrap_or(false);

        if !alive {
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

#[cfg(windows)]
fn wait_for_pid_windows(pid: u32, stop_flag: &Arc<AtomicBool>) -> Result<(), ProcessError> {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }
        .map_err(|e| ProcessError::WatchFailed(pid, e.to_string()))?;

    while !stop_flag.load(Ordering::Relaxed) {
        let res = unsafe { WaitForSingleObject(handle, 500) };
        if res == WAIT_OBJECT_0 {
            break;
        }
        if res != WAIT_TIMEOUT {
            break;
        }
    }

    unsafe {
        let _ = CloseHandle(handle);
    }
    Ok(())
}
