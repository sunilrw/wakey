use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

const MAX_IPC_LINE_BYTES: u64 = 64 * 1024;
const IPC_READ_TIMEOUT: Duration = Duration::from_secs(5);

use serde::{Deserialize, Serialize};

use crate::agent_watch::DetectedAgent;
use crate::bot_bridge::ControlCommand;
use crate::power::BatteryStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: String,
    pub reason: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSnapshot {
    pub awake: bool,
    pub paused: bool,
    pub reason: Option<String>,
    pub keep_display_on: bool,
    pub remaining_seconds: Option<u64>,
    #[serde(default)]
    pub trigger_kind: String,
    #[serde(default)]
    pub sleep_at_clock: Option<String>,
    pub battery: Option<BatteryInfo>,
    pub detected_agents: Vec<DetectedAgent>,
    pub active_schedule: Option<String>,
    pub recent_audit: Vec<AuditEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryInfo {
    pub on_ac_power: bool,
    pub percent: Option<u8>,
}

impl From<BatteryStatus> for BatteryInfo {
    fn from(b: BatteryStatus) -> Self {
        Self {
            on_ac_power: b.on_ac_power,
            percent: b.percent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    pub message: String,
    pub snapshot: Option<DaemonSnapshot>,
}

#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let base = if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        std::path::PathBuf::from(dir).join("wakey")
    } else {
        dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("wakey")
    };
    if std::fs::create_dir_all(&base).is_ok() {
        let _ = std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700));
    }
    base.join("wakey.sock")
}

#[cfg(windows)]
const WINDOWS_IPC_ADDR: &str = "127.0.0.1:47842";

pub fn spawn_ipc_server(cmd_tx: Sender<(ControlCommand, Sender<String>)>) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        let path = socket_path();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)?;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(IPC_READ_TIMEOUT));
                handle_stream(stream, &cmd_tx);
            }
        });
        return Ok(());
    }

    #[cfg(windows)]
    {
        use std::net::TcpListener;
        let listener = TcpListener::bind(WINDOWS_IPC_ADDR)?;
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(IPC_READ_TIMEOUT));
                handle_stream(stream, &cmd_tx);
            }
        });
        return Ok(());
    }
}

fn handle_stream<S: std::io::Read + std::io::Write>(
    mut stream: S,
    cmd_tx: &Sender<(ControlCommand, Sender<String>)>,
) {
    let mut reader = BufReader::new(&mut stream).take(MAX_IPC_LINE_BYTES + 1);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.len() as u64 > MAX_IPC_LINE_BYTES {
        return;
    }
    let Ok(cmd) = serde_json::from_str::<ControlCommand>(line.trim()) else {
        let _ = writeln!(
            stream,
            "{}",
            serde_json::to_string(&IpcResponse {
                ok: false,
                message: "invalid IPC command".into(),
                snapshot: None,
            })
            .unwrap()
        );
        return;
    };

    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    let _ = cmd_tx.send((cmd, reply_tx));
    let reply = reply_rx.recv_timeout(Duration::from_secs(2)).unwrap_or_else(|_| {
        serde_json::to_string(&IpcResponse {
            ok: false,
            message: "daemon timeout".into(),
            snapshot: None,
        })
        .unwrap_or_default()
    });

    let _ = writeln!(stream, "{reply}");
}

pub fn send_ipc_command(cmd: &ControlCommand) -> Result<IpcResponse, String> {
    let serialized = serde_json::to_string(cmd).map_err(|e| e.to_string())?;

    #[cfg(unix)]
    let mut stream = {
        use std::os::unix::net::UnixStream;
        UnixStream::connect(socket_path())
            .map_err(|e| format!("wakey daemon is not running ({e})"))?
    };

    #[cfg(windows)]
    let mut stream = {
        use std::net::TcpStream;
        TcpStream::connect(WINDOWS_IPC_ADDR)
            .map_err(|e| format!("wakey daemon is not running ({e})"))?
    };

    writeln!(stream, "{serialized}").map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;

    serde_json::from_str::<IpcResponse>(line.trim())
        .or_else(|_| {
            // Handle raw snapshot response if returned directly
            serde_json::from_str::<DaemonSnapshot>(line.trim()).map(|snap| IpcResponse {
                ok: true,
                message: "ok".into(),
                snapshot: Some(snap),
            })
        })
        .map_err(|e| format!("invalid response from daemon: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_serde_defaults_missing_trigger_fields() {
        let legacy_json = r#"{
            "awake": true,
            "paused": false,
            "reason": "Manual session",
            "keep_display_on": false,
            "remaining_seconds": 60,
            "battery": null,
            "detected_agents": [],
            "active_schedule": null,
            "recent_audit": []
        }"#;
        let snap: DaemonSnapshot = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(snap.trigger_kind, "");
        assert_eq!(snap.sleep_at_clock, None);
    }

    #[test]
    fn snapshot_serde_roundtrip_with_trigger_fields() {
        let snap = DaemonSnapshot {
            awake: true,
            paused: false,
            reason: Some("Manual session".into()),
            keep_display_on: true,
            remaining_seconds: Some(3600),
            trigger_kind: "manual".into(),
            sleep_at_clock: Some("13:45".into()),
            battery: None,
            detected_agents: vec![],
            active_schedule: None,
            recent_audit: vec![],
        };
        let json = serde_json::to_string(&snap).unwrap();
        let decoded: DaemonSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.trigger_kind, "manual");
        assert_eq!(decoded.sleep_at_clock.as_deref(), Some("13:45"));
    }
}
