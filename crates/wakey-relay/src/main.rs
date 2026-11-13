use std::collections::HashMap;
use std::io::Read;
use std::sync::mpsc::{self, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_BODY_BYTES: usize = 64 * 1024;
const ENTRY_TTL: Duration = Duration::from_secs(120);
const PAIR_ONLINE_TTL: Duration = Duration::from_secs(30);
const STATUS_ONLINE_TTL: Duration = Duration::from_secs(15);
const MAX_STORE_ENTRIES: usize = 10_000;
const MAX_ALLOWED_SECRETS: usize = 64;
const MAX_PENDING_COMMANDS: usize = 64;
const MAX_PAIR_FAILURES_PER_TARGET: usize = 5;
const MAX_PAIR_FAILURES_PER_IP: usize = 20;
const PAIR_RATE_WINDOW: Duration = Duration::from_secs(60);
const WORKER_COUNT: usize = 8;
const QUEUE_CAPACITY: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HeartbeatRequest {
    device_id: String,
    device_name: String,
    passcode: String,
    #[serde(default)]
    relay_token: String,
    #[serde(default)]
    allowed_secrets: Vec<String>,
    #[serde(default)]
    revoke_secrets: Vec<String>,
    #[serde(default)]
    clear_relay_secrets: bool,
    snapshot: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PairRequest {
    target_device_id: String,
    passcode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum ControlCommand {
    Status,
    Stop,
    Pause,
    Resume,
    AwakeFor { duration: String, display: bool },
    AwakeUntil { clock: String, display: bool },
    SetDisplay { display: bool },
    Extend { duration: String },
    SkipSchedule { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CommandRequest {
    target_device_id: String,
    command: ControlCommand,
}

struct RelayDeviceEntry {
    device_id: String,
    device_name: String,
    passcode: String,
    relay_token: String,
    heartbeat_secrets: Vec<String>,
    relay_issued_secrets: Vec<String>,
    latest_snapshot: Value,
    pending_commands: Vec<ControlCommand>,
    last_seen: Instant,
}

impl RelayDeviceEntry {
    fn has_secret(&self, candidate: &str) -> bool {
        if candidate.is_empty() {
            return false;
        }
        let in_hb = self
            .heartbeat_secrets
            .iter()
            .fold(false, |acc, s| acc | constant_time_eq(s, candidate));
        let in_relay = self
            .relay_issued_secrets
            .iter()
            .fold(false, |acc, s| acc | constant_time_eq(s, candidate));
        in_hb || in_relay
    }
}

#[derive(Default)]
struct PairRateLimiter {
    per_target: HashMap<String, Vec<Instant>>,
    per_ip: HashMap<String, Vec<Instant>>,
}

impl PairRateLimiter {
    fn prune(list: &mut Vec<Instant>, now: Instant) {
        list.retain(|t| now.saturating_duration_since(*t) < PAIR_RATE_WINDOW);
    }

    fn is_limited(&mut self, ip: &str, device_id: &str, now: Instant) -> bool {
        let target_key = format!("{ip}:{device_id}");
        if let Some(list) = self.per_target.get_mut(&target_key) {
            Self::prune(list, now);
            if list.len() >= MAX_PAIR_FAILURES_PER_TARGET {
                return true;
            }
        }
        if let Some(list) = self.per_ip.get_mut(ip) {
            Self::prune(list, now);
            if list.len() >= MAX_PAIR_FAILURES_PER_IP {
                return true;
            }
        }
        false
    }

    fn record_failure(&mut self, ip: &str, device_id: &str, now: Instant) {
        let target_key = format!("{ip}:{device_id}");
        let target_list = self.per_target.entry(target_key).or_default();
        Self::prune(target_list, now);
        target_list.push(now);

        let ip_list = self.per_ip.entry(ip.to_string()).or_default();
        Self::prune(ip_list, now);
        ip_list.push(now);
    }

    fn clear_target(&mut self, ip: &str, device_id: &str) {
        let target_key = format!("{ip}:{device_id}");
        self.per_target.remove(&target_key);
    }
}

struct RelayState {
    store: Mutex<HashMap<String, RelayDeviceEntry>>,
    pair_limiter: Mutex<PairRateLimiter>,
}

impl RelayState {
    fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
            pair_limiter: Mutex::new(PairRateLimiter::default()),
        }
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let max_len = ab.len().max(bb.len());
    let mut diff = ab.len() ^ bb.len();
    for i in 0..max_len {
        let x = ab.get(i).copied().unwrap_or(0);
        let y = bb.get(i).copied().unwrap_or(0);
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

fn generate_relay_secret() -> String {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).expect("OS CSPRNG unavailable");
    let mut out = String::with_capacity(32);
    for byte in buf {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn normalize_id(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 9 {
        format!("{}-{}-{}", &digits[0..3], &digits[3..6], &digits[6..9])
    } else {
        raw.trim().to_string()
    }
}

fn evict_stale_entries(map: &mut HashMap<String, RelayDeviceEntry>, now: Instant) {
    map.retain(|_, entry| now.saturating_duration_since(entry.last_seen) <= ENTRY_TTL);
}

fn validate_command(cmd: &ControlCommand) -> Result<(), &'static str> {
    let check_field = |s: &str| -> Result<(), &'static str> {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed.len() > 128 {
            return Err("invalid command argument");
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err("invalid control character in command argument");
        }
        Ok(())
    };
    match cmd {
        ControlCommand::Status
        | ControlCommand::Stop
        | ControlCommand::Pause
        | ControlCommand::Resume
        | ControlCommand::SetDisplay { .. } => Ok(()),
        ControlCommand::AwakeFor { duration, .. } | ControlCommand::Extend { duration } => {
            check_field(duration)
        }
        ControlCommand::AwakeUntil { clock, .. } => check_field(clock),
        ControlCommand::SkipSchedule { name } => check_field(name),
    }
}

fn read_bounded_body(
    request: &mut tiny_http::Request,
    max_bytes: usize,
) -> Result<String, (u16, &'static str)> {
    if let Some(len) = request.body_length() {
        if len > max_bytes {
            return Err((413, "payload too large"));
        }
    }
    let mut buf = Vec::new();
    let limit = (max_bytes as u64).saturating_add(1);
    request
        .as_reader()
        .take(limit)
        .read_to_end(&mut buf)
        .map_err(|_| (400, "failed to read request body"))?;
    if buf.len() > max_bytes {
        return Err((413, "payload too large"));
    }
    String::from_utf8(buf).map_err(|_| (400, "request body must be valid UTF-8"))
}

fn process_heartbeat(state: &RelayState, device_secret_hdr: &str, body: &str) -> (u16, String) {
    let hb = match serde_json::from_str::<HeartbeatRequest>(body) {
        Ok(hb) => hb,
        Err(e) => {
            return (
                400,
                serde_json::json!({ "ok": false, "error": e.to_string() }).to_string(),
            )
        }
    };

    let token = if !device_secret_hdr.trim().is_empty() {
        device_secret_hdr.trim().to_string()
    } else {
        hb.relay_token.trim().to_string()
    };
    if token.is_empty() {
        return (401, r#"{"ok":false,"error":"unauthorized"}"#.to_string());
    }

    let id = normalize_id(&hb.device_id);
    if id.is_empty() {
        return (400, r#"{"ok":false,"error":"invalid device_id"}"#.to_string());
    }

    let now = Instant::now();
    let mut map = state.store.lock().unwrap_or_else(|e| e.into_inner());
    evict_stale_entries(&mut map, now);

    if let Some(entry) = map.get_mut(&id) {
        if !constant_time_eq(&entry.relay_token, &token) {
            return (401, r#"{"ok":false,"error":"unauthorized"}"#.to_string());
        }
        let passcode_changed = !constant_time_eq(&entry.passcode, &hb.passcode);
        entry.device_name = hb.device_name;
        entry.passcode = hb.passcode;
        entry.heartbeat_secrets = hb
            .allowed_secrets
            .into_iter()
            .filter(|s| !s.is_empty())
            .take(MAX_ALLOWED_SECRETS)
            .collect();
        if passcode_changed || hb.clear_relay_secrets {
            entry.relay_issued_secrets.clear();
        } else if !hb.revoke_secrets.is_empty() {
            entry.relay_issued_secrets.retain(|s| {
                !hb.revoke_secrets
                    .iter()
                    .any(|rev| constant_time_eq(s, rev))
            });
        }
        entry.latest_snapshot = hb.snapshot;
        entry.last_seen = now;
        let cmds: Vec<ControlCommand> = std::mem::take(&mut entry.pending_commands);
        return (
            200,
            serde_json::json!({ "ok": true, "pending_commands": cmds }).to_string(),
        );
    }

    if map.len() >= MAX_STORE_ENTRIES {
        return (
            503,
            r#"{"ok":false,"error":"relay capacity reached"}"#.to_string(),
        );
    }

    let heartbeat_secrets: Vec<String> = hb
        .allowed_secrets
        .into_iter()
        .filter(|s| !s.is_empty())
        .take(MAX_ALLOWED_SECRETS)
        .collect();
    map.insert(
        id.clone(),
        RelayDeviceEntry {
            device_id: id,
            device_name: hb.device_name,
            passcode: hb.passcode,
            relay_token: token,
            heartbeat_secrets,
            relay_issued_secrets: Vec::new(),
            latest_snapshot: hb.snapshot,
            pending_commands: Vec::new(),
            last_seen: now,
        },
    );

    (
        200,
        serde_json::json!({ "ok": true, "pending_commands": Vec::<ControlCommand>::new() })
            .to_string(),
    )
}

fn pair_failed_json(msg: &str) -> String {
    serde_json::json!({
        "ok": false,
        "device_id": "",
        "device_name": "",
        "shared_secret": "",
        "message": msg
    })
    .to_string()
}

fn process_pair(state: &RelayState, client_ip: &str, body: &str) -> (u16, String) {
    let req = match serde_json::from_str::<PairRequest>(body) {
        Ok(req) => req,
        Err(e) => return (400, pair_failed_json(&e.to_string())),
    };

    let id = normalize_id(&req.target_device_id);
    let now = Instant::now();

    {
        let mut limiter = state.pair_limiter.lock().unwrap_or_else(|e| e.into_inner());
        if limiter.is_limited(client_ip, &id, now) {
            return (429, pair_failed_json("too many pairing attempts"));
        }
    }

    let mut map = state.store.lock().unwrap_or_else(|e| e.into_inner());
    evict_stale_entries(&mut map, now);

    let paired = match map.get_mut(&id) {
        Some(entry)
            if now.saturating_duration_since(entry.last_seen) < PAIR_ONLINE_TTL
                && !entry.passcode.is_empty()
                && constant_time_eq(&entry.passcode, req.passcode.trim()) =>
        {
            let secret = generate_relay_secret();
            if entry.relay_issued_secrets.len() >= MAX_ALLOWED_SECRETS {
                entry.relay_issued_secrets.remove(0);
            }
            entry.relay_issued_secrets.push(secret.clone());
            Some((entry.device_id.clone(), entry.device_name.clone(), secret))
        }
        _ => None,
    };
    drop(map);

    if let Some((device_id, device_name, secret)) = paired {
        let mut limiter = state.pair_limiter.lock().unwrap_or_else(|e| e.into_inner());
        limiter.clear_target(client_ip, &id);
        (
            200,
            serde_json::json!({
                "ok": true,
                "device_id": device_id,
                "device_name": device_name,
                "shared_secret": secret,
                "message": "paired via relay"
            })
            .to_string(),
        )
    } else {
        let mut limiter = state.pair_limiter.lock().unwrap_or_else(|e| e.into_inner());
        limiter.record_failure(client_ip, &id, now);
        (200, pair_failed_json("pairing failed"))
    }
}

fn process_status(state: &RelayState, raw_id: &str, secret_hdr: &str) -> (u16, String) {
    let id = normalize_id(raw_id);
    let now = Instant::now();
    let mut map = state.store.lock().unwrap_or_else(|e| e.into_inner());
    evict_stale_entries(&mut map, now);

    match map.get(&id) {
        Some(entry) if entry.has_secret(secret_hdr) => {
            let online = now.saturating_duration_since(entry.last_seen) < STATUS_ONLINE_TTL;
            (
                200,
                serde_json::json!({
                    "device_id": entry.device_id,
                    "name": entry.device_name,
                    "online": online,
                    "via": "relay",
                    "mac_address": Value::Null,
                    "snapshot": entry.latest_snapshot,
                    "error": if online { Value::Null } else { Value::String("stale".into()) }
                })
                .to_string(),
            )
        }
        Some(_) => (401, r#"{"ok":false,"error":"unauthorized"}"#.to_string()),
        None => (404, r#"{"ok":false,"error":"device offline"}"#.to_string()),
    }
}

fn process_command(state: &RelayState, secret_hdr: &str, body: &str) -> (u16, String) {
    let req = match serde_json::from_str::<CommandRequest>(body) {
        Ok(req) => req,
        Err(e) => {
            return (
                400,
                serde_json::json!({ "ok": false, "message": e.to_string() }).to_string(),
            )
        }
    };

    if let Err(msg) = validate_command(&req.command) {
        return (
            400,
            serde_json::json!({ "ok": false, "message": msg }).to_string(),
        );
    }

    let id = normalize_id(&req.target_device_id);
    let now = Instant::now();
    let mut map = state.store.lock().unwrap_or_else(|e| e.into_inner());
    evict_stale_entries(&mut map, now);

    match map.get_mut(&id) {
        Some(entry) if entry.has_secret(secret_hdr) => {
            if entry.pending_commands.len() >= MAX_PENDING_COMMANDS {
                return (
                    429,
                    r#"{"ok":false,"message":"command queue full"}"#.to_string(),
                );
            }
            entry.pending_commands.push(req.command);
            (
                200,
                r#"{"ok":true,"message":"queued on relay"}"#.to_string(),
            )
        }
        Some(_) => (401, r#"{"ok":false,"message":"unauthorized"}"#.to_string()),
        None => (404, r#"{"ok":false,"message":"device offline"}"#.to_string()),
    }
}

fn handle_request(mut request: tiny_http::Request, state: &RelayState) {
    let url = request.url().to_string();
    let method = request.method().as_str().to_uppercase();
    let client_ip = request
        .remote_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut secret_hdr = String::new();
    let mut device_secret_hdr = String::new();
    for h in request.headers() {
        if h.field.equiv("X-Wakey-Peer-Secret") {
            secret_hdr = h.value.as_str().to_string();
        } else if h.field.equiv("X-Wakey-Device-Secret") {
            device_secret_hdr = h.value.as_str().to_string();
        }
    }

    let ct = tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();

    let (status_code, resp_body) = if method == "GET" && url == "/health" {
        (200, r#"{"ok":true,"service":"wakey-relay"}"#.to_string())
    } else if method == "POST" && url == "/v1/heartbeat" {
        match read_bounded_body(&mut request, MAX_BODY_BYTES) {
            Ok(body) => process_heartbeat(state, &device_secret_hdr, &body),
            Err((code, msg)) => (
                code,
                serde_json::json!({ "ok": false, "error": msg }).to_string(),
            ),
        }
    } else if method == "POST" && url == "/v1/pair" {
        match read_bounded_body(&mut request, MAX_BODY_BYTES) {
            Ok(body) => process_pair(state, &client_ip, &body),
            Err((code, msg)) => (code, pair_failed_json(msg)),
        }
    } else if method == "GET" && url.starts_with("/v1/status/") {
        let raw_id = url.strip_prefix("/v1/status/").unwrap_or("");
        process_status(state, raw_id, &secret_hdr)
    } else if method == "POST" && url == "/v1/command" {
        match read_bounded_body(&mut request, MAX_BODY_BYTES) {
            Ok(body) => process_command(state, &secret_hdr, &body),
            Err((code, msg)) => (
                code,
                serde_json::json!({ "ok": false, "message": msg }).to_string(),
            ),
        }
    } else {
        (404, r#"{"ok":false,"message":"not found"}"#.to_string())
    };

    let _ = request.respond(
        tiny_http::Response::from_string(resp_body)
            .with_status_code(status_code)
            .with_header(ct),
    );
}

fn main() {
    let listen_addr = std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with('-'))
        .or_else(|| std::env::var("WAKEY_RELAY_LISTEN").ok())
        .or_else(|| std::env::var("PORT").ok().map(|p| format!("0.0.0.0:{p}")))
        .unwrap_or_else(|| "0.0.0.0:7845".to_string());

    let server = tiny_http::Server::http(&listen_addr)
        .unwrap_or_else(|e| panic!("failed to bind relay at {listen_addr}: {e}"));
    eprintln!("wakey-relay listening on {listen_addr} (blind zero-login mode)");

    let state = Arc::new(RelayState::new());
    let (tx, rx) = mpsc::sync_channel::<tiny_http::Request>(QUEUE_CAPACITY);
    let rx = Arc::new(Mutex::new(rx));

    for _ in 0..WORKER_COUNT {
        let rx_worker = Arc::clone(&rx);
        let state_worker = Arc::clone(&state);
        thread::spawn(move || loop {
            let req = {
                let guard = rx_worker.lock().unwrap_or_else(|e| e.into_inner());
                guard.recv()
            };
            match req {
                Ok(request) => handle_request(request, &state_worker),
                Err(_) => break,
            }
        });
    }

    for request in server.incoming_requests() {
        if let Err(TrySendError::Full(req)) | Err(TrySendError::Disconnected(req)) =
            tx.try_send(request)
        {
            if let Ok(ct) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
                let _ = req.respond(
                    tiny_http::Response::from_string(r#"{"ok":false,"error":"server busy"}"#)
                        .with_status_code(503)
                        .with_header(ct),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hb_json(id: &str, passcode: &str, token: &str, allowed: &[&str]) -> String {
        serde_json::json!({
            "device_id": id,
            "device_name": "test-pc",
            "passcode": passcode,
            "relay_token": token,
            "allowed_secrets": allowed,
            "snapshot": {"awake": true}
        })
        .to_string()
    }

    #[test]
    fn secret_generation_uses_csprng_hex() {
        let s1 = generate_relay_secret();
        let s2 = generate_relay_secret();
        assert_eq!(s1.len(), 32);
        assert_eq!(s2.len(), 32);
        assert!(s1.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(s1, s2);
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc", "abcd"));
    }

    #[test]
    fn heartbeat_requires_token_and_prevents_hijack() {
        let state = RelayState::new();

        // Missing token rejected
        let (code, _) = process_heartbeat(&state, "", &hb_json("123456789", "111111", "", &[]));
        assert_eq!(code, 401);

        // First heartbeat registers token
        let (code, _) = process_heartbeat(
            &state,
            "tok-owner",
            &hb_json("123456789", "111111", "", &["peer-1"]),
        );
        assert_eq!(code, 200);

        // Queue a command for the device
        let cmd_body = serde_json::json!({
            "target_device_id": "123-456-789",
            "command": { "cmd": "status" }
        })
        .to_string();
        let (code, _) = process_command(&state, "peer-1", &cmd_body);
        assert_eq!(code, 200);

        // Attacker with wrong token cannot hijack or drain pending_commands
        let (code, _) = process_heartbeat(
            &state,
            "tok-attacker",
            &hb_json("123-456-789", "999999", "", &["evil-sec"]),
        );
        assert_eq!(code, 401);

        // Legitimate owner still receives queued command and old passcode is intact
        let (code, body) = process_heartbeat(
            &state,
            "tok-owner",
            &hb_json("123-456-789", "111111", "", &["peer-1"]),
        );
        assert_eq!(code, 200);
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["pending_commands"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn heartbeat_syncs_and_revokes_allowed_secrets() {
        let state = RelayState::new();
        let (code, _) = process_heartbeat(
            &state,
            "tok",
            &hb_json("123456789", "111111", "", &["sec-a", "sec-b"]),
        );
        assert_eq!(code, 200);

        let (code, _) = process_status(&state, "123-456-789", "sec-b");
        assert_eq!(code, 200);

        // Remove sec-b from heartbeat -> revoked immediately
        let (code, _) = process_heartbeat(
            &state,
            "tok",
            &hb_json("123456789", "111111", "", &["sec-a"]),
        );
        assert_eq!(code, 200);

        let (code, _) = process_status(&state, "123-456-789", "sec-b");
        assert_eq!(code, 401);
        let (code, _) = process_status(&state, "123-456-789", "sec-a");
        assert_eq!(code, 200);
    }

    #[test]
    fn pair_rate_limits_and_hides_enumeration() {
        let state = RelayState::new();
        let _ = process_heartbeat(&state, "tok", &hb_json("123456789", "654321", "", &[]));

        let bad_online = serde_json::json!({
            "target_device_id": "123-456-789",
            "passcode": "000000"
        })
        .to_string();
        let bad_offline = serde_json::json!({
            "target_device_id": "999-999-999",
            "passcode": "000000"
        })
        .to_string();

        let (c1, b1) = process_pair(&state, "10.0.0.1", &bad_online);
        let (c2, b2) = process_pair(&state, "10.0.0.2", &bad_offline);
        assert_eq!(c1, 200);
        assert_eq!(c2, 200);
        let v1: Value = serde_json::from_str(&b1).unwrap();
        let v2: Value = serde_json::from_str(&b2).unwrap();
        assert_eq!(v1["message"], "pairing failed");
        assert_eq!(v2["message"], "pairing failed");

        // Trigger rate limit on 10.0.0.1 for 123-456-789 (1 failure already recorded above)
        for _ in 0..4 {
            let (c, _) = process_pair(&state, "10.0.0.1", &bad_online);
            assert_eq!(c, 200);
        }
        let (c_limited, _) = process_pair(&state, "10.0.0.1", &bad_online);
        assert_eq!(c_limited, 429);
    }

    #[test]
    fn command_validates_schema_and_queue_cap() {
        let state = RelayState::new();
        let _ = process_heartbeat(
            &state,
            "tok",
            &hb_json("123456789", "654321", "", &["peer-sec"]),
        );

        let invalid_cmd = serde_json::json!({
            "target_device_id": "123-456-789",
            "command": { "cmd": "arbitrary_shell_exec", "arg": "rm -rf /" }
        })
        .to_string();
        let (code, _) = process_command(&state, "peer-sec", &invalid_cmd);
        assert_eq!(code, 400);

        let valid_cmd = serde_json::json!({
            "target_device_id": "123-456-789",
            "command": { "cmd": "awake_for", "duration": "30m", "display": false }
        })
        .to_string();
        for _ in 0..MAX_PENDING_COMMANDS {
            let (code, _) = process_command(&state, "peer-sec", &valid_cmd);
            assert_eq!(code, 200);
        }
        let (code, _) = process_command(&state, "peer-sec", &valid_cmd);
        assert_eq!(code, 429);
    }
}
