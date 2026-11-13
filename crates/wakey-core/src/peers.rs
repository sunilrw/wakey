use std::collections::HashMap;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::bot_bridge::ControlCommand;
use crate::config::{constant_time_eq, generate_shared_secret, Config, PairedPeer, PeersConfig};
use crate::ipc::DaemonSnapshot;

const MAX_PEER_BODY_BYTES: usize = 64 * 1024;
const PAIR_MAX_FAILURES: u32 = 5;
const PAIR_LOCKOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedPairingCode {
    pub device_id: String,
    pub passcode: Option<String>,
    pub name: Option<String>,
    pub lan_addr: Option<String>,
    pub relay_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequest {
    pub target_device_id: Option<String>,
    pub passcode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairResponse {
    pub ok: bool,
    pub device_id: String,
    pub device_name: String,
    pub shared_secret: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePeerStatus {
    pub device_id: String,
    pub name: String,
    pub online: bool,
    pub via: String,
    pub mac_address: Option<String>,
    pub snapshot: Option<DaemonSnapshot>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayHeartbeatRequest {
    pub device_id: String,
    pub device_name: String,
    pub passcode: String,
    #[serde(default)]
    pub relay_token: String,
    pub allowed_secrets: Vec<String>,
    pub snapshot: DaemonSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayHeartbeatResponse {
    pub ok: bool,
    pub pending_commands: Vec<ControlCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayCommandRequest {
    pub target_device_id: String,
    pub command: ControlCommand,
}

pub fn normalize_device_id(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 9 {
        format!("{}-{}-{}", &digits[0..3], &digits[3..6], &digits[6..9])
    } else {
        raw.trim().to_string()
    }
}

pub fn detect_local_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    Some(sock.local_addr().ok()?.ip())
}

fn pct_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn pct_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Builds a single copy-pasteable pairing string for the other PC.
pub fn format_pairing_clipboard(peers: &PeersConfig) -> String {
    let lan = detect_local_ip()
        .map(|ip| format!("{ip}:{}", peers.listen_port))
        .unwrap_or_default();
    let mut s = format!(
        "wakey://pair/{}/{}?name={}",
        peers.device_id,
        peers.passcode,
        pct_encode(&peers.device_name)
    );
    if !lan.is_empty() {
        s.push_str(&format!("&lan={}", pct_encode(&lan)));
    }
    if !peers.relay_url.trim().is_empty() {
        s.push_str(&format!("&relay={}", pct_encode(peers.relay_url.trim())));
    }
    s
}

pub fn validate_lan_addr(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
        || trimmed.contains(['/', '?', '#', '@', '\\'])
    {
        return Err("invalid LAN address".into());
    }
    if let Ok(sock) = trimmed.parse::<SocketAddr>() {
        if let IpAddr::V4(v4) = sock.ip() {
            if v4.octets()[0] == 169 && v4.octets()[1] == 254 {
                return Err("link-local metadata IP is not allowed".into());
            }
        }
        return Ok(trimmed.to_string());
    }
    let Some((host, port_str)) = trimmed.rsplit_once(':') else {
        return Err("LAN address must be host:port".into());
    };
    let port: u16 = port_str
        .parse()
        .map_err(|_| "invalid LAN port".to_string())?;
    if port == 0
        || host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err("invalid LAN host".into());
    }
    Ok(trimmed.to_string())
}

pub fn validate_relay_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("invalid relay URL".into());
    }
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .ok_or_else(|| "relay URL must use http:// or https://".to_string())?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if authority.starts_with('[') {
        authority
            .split(']')
            .next()
            .unwrap_or("")
            .trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        return Err("relay URL missing host".into());
    }
    let lower_host = host.to_lowercase();
    if lower_host == "metadata.google.internal" || lower_host.starts_with("169.254.") {
        return Err("relay URL cannot point to link-local metadata address".into());
    }
    if let Ok(IpAddr::V4(v4)) = host.parse::<IpAddr>() {
        if v4.octets()[0] == 169 && v4.octets()[1] == 254 {
            return Err("relay URL cannot point to link-local metadata address".into());
        }
    }
    Ok(trimmed.to_string())
}

/// Parses either a `wakey://pair/...` URI or `ID:PASSCODE` / `ID PASSCODE` / bare `ID`.
pub fn parse_pairing_input(input: &str) -> Result<ParsedPairingCode, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("pairing code is empty".into());
    }

    if let Some(rest) = trimmed.strip_prefix("wakey://pair/") {
        let (path_part, query_part) = rest
            .split_once('?')
            .map(|(p, q)| (p, Some(q)))
            .unwrap_or((rest, None));

        let segments: Vec<&str> = path_part.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return Err("invalid wakey://pair URI: missing device ID".into());
        }
        let device_id = normalize_device_id(segments[0]);
        let passcode = segments.get(1).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

        let mut name = None;
        let mut lan_addr = None;
        let mut relay_url = None;

        if let Some(q) = query_part {
            for kv in q.split('&') {
                if let Some((k, v)) = kv.split_once('=') {
                    let val = pct_decode(v);
                    if val.is_empty() {
                        continue;
                    }
                    match k {
                        "name" => name = Some(val),
                        "lan" => lan_addr = Some(validate_lan_addr(&val)?),
                        "relay" => relay_url = Some(validate_relay_url(&val)?),
                        _ => {}
                    }
                }
            }
        }

        return Ok(ParsedPairingCode {
            device_id,
            passcode,
            name,
            lan_addr,
            relay_url,
        });
    }

    // Plain formats: "482-910-334:849201" or "482-910-334 849201" or "482-910-334"
    let normalized = trimmed.replace(':', " ");
    let parts: Vec<&str> = normalized.split_whitespace().collect();
    if parts.is_empty() {
        return Err("invalid pairing code".into());
    }
    let device_id = normalize_device_id(parts[0]);
    if device_id.len() < 9 {
        return Err("device ID should be 9 digits (e.g. 482-910-334)".into());
    }
    let passcode = parts.get(1).map(|s| s.trim().to_string());

    Ok(ParsedPairingCode {
        device_id,
        passcode,
        name: None,
        lan_addr: None,
        relay_url: None,
    })
}

fn secret_authorized(cfg: &PeersConfig, header_val: &str) -> bool {
    if header_val.is_empty() {
        return false;
    }
    cfg.incoming_secrets
        .iter()
        .any(|s| !s.is_empty() && constant_time_eq(s, header_val))
}

fn read_limited_body(request: &mut tiny_http::Request, max_bytes: usize) -> Result<String, u16> {
    if request
        .body_length()
        .map(|len| len > max_bytes)
        .unwrap_or(false)
    {
        return Err(413);
    }
    let mut body = String::new();
    request
        .as_reader()
        .take((max_bytes + 1) as u64)
        .read_to_string(&mut body)
        .map_err(|_| 400u16)?;
    if body.len() > max_bytes {
        return Err(413);
    }
    Ok(body)
}

/// Starts the local peer-to-peer HTTP server on `0.0.0.0:<listen_port>` for direct LAN pairing & control.
pub fn spawn_peer_server(
    listen_port: u16,
    cmd_tx: Sender<(ControlCommand, Sender<String>)>,
    shared_snapshot: Arc<Mutex<DaemonSnapshot>>,
) {
    let addr = format!("0.0.0.0:{listen_port}");
    let Ok(server) = tiny_http::Server::http(&addr) else {
        // Non-fatal if port is already in use (e.g., CLI daemon + tray running together).
        return;
    };

    thread::spawn(move || {
        let mut pair_failures: HashMap<IpAddr, (u32, Instant)> = HashMap::new();

        for mut request in server.incoming_requests() {
            let url = request.url().to_string();
            let method = request.method().as_str().to_uppercase();
            let peer_ip = request.remote_addr().map(|a| a.ip());
            let secret_hdr = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("X-Wakey-Peer-Secret"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default();

            let mut cfg = Config::load_or_default().unwrap_or_default();
            let ct = tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();

            if method == "POST" && url == "/peer/pair" {
                if let Some(ip) = peer_ip {
                    if let Some((count, first_at)) = pair_failures.get(&ip) {
                        if first_at.elapsed() < Duration::from_secs(PAIR_LOCKOUT_SECS)
                            && *count >= PAIR_MAX_FAILURES
                        {
                            let _ = request.respond(
                                tiny_http::Response::from_string(
                                    r#"{"ok":false,"device_id":"","device_name":"","shared_secret":"","message":"too many failed pairing attempts; try again later"}"#,
                                )
                                .with_status_code(429)
                                .with_header(ct),
                            );
                            continue;
                        }
                    }
                }

                let body = match read_limited_body(&mut request, MAX_PEER_BODY_BYTES) {
                    Ok(b) => b,
                    Err(code) => {
                        let _ = request.respond(
                            tiny_http::Response::from_string(
                                r#"{"ok":false,"device_id":"","device_name":"","shared_secret":"","message":"invalid or oversized request"}"#,
                            )
                            .with_status_code(code)
                            .with_header(ct),
                        );
                        continue;
                    }
                };

                let resp_json = match serde_json::from_str::<PairRequest>(&body) {
                    Ok(req) => {
                        if !cfg.peers.passcode.is_empty()
                            && constant_time_eq(req.passcode.trim(), &cfg.peers.passcode)
                        {
                            if let Some(ip) = peer_ip {
                                pair_failures.remove(&ip);
                            }
                            let secret = generate_shared_secret();
                            cfg.peers.incoming_secrets.push(secret.clone());
                            let _ = cfg.save();
                            serde_json::to_string(&PairResponse {
                                ok: true,
                                device_id: cfg.peers.device_id.clone(),
                                device_name: cfg.peers.device_name.clone(),
                                shared_secret: secret,
                                message: Some("paired".into()),
                            })
                            .unwrap()
                        } else {
                            if let Some(ip) = peer_ip {
                                let entry = pair_failures
                                    .entry(ip)
                                    .or_insert_with(|| (0, Instant::now()));
                                if entry.1.elapsed() >= Duration::from_secs(PAIR_LOCKOUT_SECS) {
                                    *entry = (1, Instant::now());
                                } else {
                                    entry.0 = entry.0.saturating_add(1);
                                }
                            }
                            serde_json::to_string(&PairResponse {
                                ok: false,
                                device_id: cfg.peers.device_id.clone(),
                                device_name: cfg.peers.device_name.clone(),
                                shared_secret: String::new(),
                                message: Some("invalid passcode".into()),
                            })
                            .unwrap()
                        }
                    }
                    Err(e) => serde_json::to_string(&PairResponse {
                        ok: false,
                        device_id: String::new(),
                        device_name: String::new(),
                        shared_secret: String::new(),
                        message: Some(format!("bad request: {e}")),
                    })
                    .unwrap(),
                };
                let _ = request.respond(tiny_http::Response::from_string(resp_json).with_header(ct));
                continue;
            }

            if !secret_authorized(&cfg.peers, &secret_hdr) {
                let _ = request.respond(
                    tiny_http::Response::from_string(r#"{"ok":false,"error":"unauthorized"}"#)
                        .with_status_code(401)
                        .with_header(ct),
                );
                continue;
            }

            if method == "GET" && url == "/peer/status" {
                let snap = shared_snapshot
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let payload = RemotePeerStatus {
                    device_id: cfg.peers.device_id.clone(),
                    name: cfg.peers.device_name.clone(),
                    online: true,
                    via: "lan".into(),
                    mac_address: None,
                    snapshot: Some(snap),
                    error: None,
                };
                let _ = request.respond(
                    tiny_http::Response::from_string(serde_json::to_string(&payload).unwrap())
                        .with_header(ct),
                );
                continue;
            }

            if method == "POST" && url == "/peer/command" {
                let body = match read_limited_body(&mut request, MAX_PEER_BODY_BYTES) {
                    Ok(b) => b,
                    Err(code) => {
                        let _ = request.respond(
                            tiny_http::Response::from_string(
                                r#"{"ok":false,"message":"invalid or oversized request"}"#,
                            )
                            .with_status_code(code)
                            .with_header(ct),
                        );
                        continue;
                    }
                };
                if let Ok(cmd) = serde_json::from_str::<ControlCommand>(&body) {
                    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
                    let _ = cmd_tx.send((cmd, reply_tx));
                    let reply = reply_rx
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap_or_else(|_| r#"{"ok":false,"message":"timeout"}"#.into());
                    let _ = request.respond(tiny_http::Response::from_string(reply).with_header(ct));
                } else {
                    let _ = request.respond(
                        tiny_http::Response::from_string(r#"{"ok":false,"message":"bad command"}"#)
                            .with_status_code(400)
                            .with_header(ct),
                    );
                }
                continue;
            }

            let _ = request.respond(
                tiny_http::Response::from_string(r#"{"ok":false,"message":"not found"}"#)
                    .with_status_code(404)
                    .with_header(ct),
            );
        }
    });
}

/// Periodically sends heartbeats to the configured VPS relay and executes queued remote commands.
pub fn spawn_relay_heartbeat(
    cmd_tx: Sender<(ControlCommand, Sender<String>)>,
    shared_snapshot: Arc<Mutex<DaemonSnapshot>>,
) {
    thread::spawn(move || {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(4))
            .build();
        loop {
            let cfg = Config::load_or_default().unwrap_or_default();
            let relay = cfg.peers.relay_url.trim().trim_end_matches('/').to_string();
            if !relay.is_empty() && !cfg.peers.device_id.is_empty() {
                let snap = shared_snapshot
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let req_body = RelayHeartbeatRequest {
                    device_id: cfg.peers.device_id.clone(),
                    device_name: cfg.peers.device_name.clone(),
                    passcode: cfg.peers.passcode.clone(),
                    relay_token: cfg.peers.relay_token.clone(),
                    allowed_secrets: cfg.peers.incoming_secrets.clone(),
                    snapshot: snap,
                };
                if let Ok(resp) = agent
                    .post(&format!("{relay}/v1/heartbeat"))
                    .set("X-Wakey-Device-Secret", &cfg.peers.relay_token)
                    .send_json(serde_json::to_value(&req_body).unwrap_or_default())
                {
                    if let Ok(hb) = resp.into_json::<RelayHeartbeatResponse>() {
                        for cmd in hb.pending_commands {
                            let (reply_tx, _reply_rx) = std::sync::mpsc::channel();
                            let _ = cmd_tx.send((cmd, reply_tx));
                        }
                    }
                }
            }
            thread::sleep(Duration::from_secs(3));
        }
    });
}

/// Pairs with a remote PC using a pasted `wakey://pair/...` code or `ID` + `passcode`.
pub fn pair_with_remote(
    cfg: &mut Config,
    raw_input: &str,
    extra_passcode: Option<&str>,
) -> Result<PairedPeer, String> {
    let parsed = parse_pairing_input(raw_input)?;
    let passcode = parsed
        .passcode
        .or_else(|| extra_passcode.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        .ok_or_else(|| "passcode is required to pair".to_string())?;

    if parsed.device_id == cfg.peers.device_id {
        return Err("cannot pair a computer with itself".into());
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .build();

    let mut paired_via_lan = false;
    let mut pair_resp: Option<PairResponse> = None;

    // 1. Try direct LAN first if present
    if let Some(ref lan) = parsed.lan_addr {
        let url = format!("http://{lan}/peer/pair");
        if let Ok(r) = agent.post(&url).send_json(serde_json::json!({
            "target_device_id": parsed.device_id,
            "passcode": passcode
        })) {
            if let Ok(pr) = r.into_json::<PairResponse>() {
                if pr.ok {
                    paired_via_lan = true;
                    pair_resp = Some(pr);
                } else {
                    return Err(pr.message.unwrap_or_else(|| "pairing rejected".into()));
                }
            }
        }
    }

    // 2. Fallback to VPS relay (from code or local config)
    let relay = parsed
        .relay_url
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(cfg.peers.relay_url.trim())
        .trim_end_matches('/')
        .to_string();

    if pair_resp.is_none() && !relay.is_empty() {
        let url = format!("{relay}/v1/pair");
        let resp = agent
            .post(&url)
            .send_json(serde_json::json!({
                "target_device_id": parsed.device_id,
                "passcode": passcode
            }))
            .map_err(|e| format!("could not reach relay at {relay}: {e}"))?;
        let pr = resp
            .into_json::<PairResponse>()
            .map_err(|e| format!("invalid relay pair response: {e}"))?;
        if !pr.ok {
            return Err(pr.message.unwrap_or_else(|| "pairing rejected by relay".into()));
        }
        pair_resp = Some(pr);
    }

    let pr = pair_resp.ok_or_else(|| {
        "could not reach target PC on LAN and no VPS relay URL is configured".to_string()
    })?;

    let peer = PairedPeer {
        device_id: normalize_device_id(&pr.device_id),
        name: parsed.name.unwrap_or(pr.device_name),
        shared_secret: pr.shared_secret,
        direct_addr: if paired_via_lan {
            parsed.lan_addr
        } else {
            None
        },
        mac_address: None,
    };

    cfg.peers.paired_peers.retain(|p| p.device_id != peer.device_id);
    cfg.peers.paired_peers.push(peer.clone());
    cfg.save().map_err(|e| e.to_string())?;
    Ok(peer)
}

pub fn fetch_paired_peers_status(cfg: &Config) -> Vec<RemotePeerStatus> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(1200))
        .build();
    let relay = cfg.peers.relay_url.trim().trim_end_matches('/');

    cfg.peers
        .paired_peers
        .iter()
        .map(|peer| {
            // Try direct LAN
            if let Some(ref addr) = peer.direct_addr {
                let url = format!("http://{addr}/peer/status");
                if let Ok(r) = agent
                    .get(&url)
                    .set("X-Wakey-Peer-Secret", &peer.shared_secret)
                    .call()
                {
                    if let Ok(mut st) = r.into_json::<RemotePeerStatus>() {
                        st.name = peer.name.clone();
                        st.mac_address = peer.mac_address.clone();
                        return st;
                    }
                }
            }

            // Try VPS relay
            if !relay.is_empty() {
                let url = format!("{relay}/v1/status/{}", peer.device_id);
                if let Ok(r) = agent
                    .get(&url)
                    .set("X-Wakey-Peer-Secret", &peer.shared_secret)
                    .call()
                {
                    if let Ok(mut st) = r.into_json::<RemotePeerStatus>() {
                        st.name = peer.name.clone();
                        st.mac_address = peer.mac_address.clone();
                        return st;
                    }
                }
            }

            RemotePeerStatus {
                device_id: peer.device_id.clone(),
                name: peer.name.clone(),
                online: false,
                via: "offline".into(),
                mac_address: peer.mac_address.clone(),
                snapshot: None,
                error: Some("unreachable".into()),
            }
        })
        .collect()
}

pub fn send_remote_peer_command(
    cfg: &Config,
    target_device_id: &str,
    cmd: &ControlCommand,
) -> Result<String, String> {
    let peer = cfg
        .peers
        .paired_peers
        .iter()
        .find(|p| p.device_id == target_device_id)
        .ok_or_else(|| format!("peer '{target_device_id}' not found"))?;

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(2))
        .build();
    let body = serde_json::to_value(cmd).unwrap_or_default();

    if let Some(ref addr) = peer.direct_addr {
        let url = format!("http://{addr}/peer/command");
        if let Ok(r) = agent
            .post(&url)
            .set("X-Wakey-Peer-Secret", &peer.shared_secret)
            .send_json(body.clone())
        {
            if let Ok(txt) = r.into_string() {
                return Ok(txt);
            }
        }
    }

    let relay = cfg.peers.relay_url.trim().trim_end_matches('/');
    if !relay.is_empty() {
        let url = format!("{relay}/v1/command");
        let resp = agent
            .post(&url)
            .set("X-Wakey-Peer-Secret", &peer.shared_secret)
            .send_json(serde_json::json!({
                "target_device_id": peer.device_id,
                "command": cmd
            }))
            .map_err(|e| format!("relay command failed: {e}"))?;
        return resp.into_string().map_err(|e| e.to_string());
    }

    Err("peer is unreachable on LAN and no relay is configured".into())
}

/// Sends a standard Wake-on-LAN UDP magic packet (`FF*6 + MAC*16`) to `255.255.255.255:9`.
pub fn send_wake_on_lan(mac_str: &str) -> Result<(), String> {
    let clean: Vec<&str> = mac_str
        .trim()
        .split([':', '-'])
        .filter(|s| !s.is_empty())
        .collect();
    if clean.len() != 6 {
        return Err("MAC address must have 6 hex bytes (e.g. AA:BB:CC:DD:EE:FF)".into());
    }
    let mut mac = [0u8; 6];
    for (i, part) in clean.iter().enumerate() {
        mac[i] = u8::from_str_radix(part, 16)
            .map_err(|_| format!("invalid hex byte '{part}' in MAC address"))?;
    }

    let mut packet = [0xFFu8; 102];
    for i in 0..16 {
        let offset = 6 + i * 6;
        packet[offset..offset + 6].copy_from_slice(&mac);
    }

    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    sock.set_broadcast(true).map_err(|e| e.to_string())?;
    sock.send_to(&packet, "255.255.255.255:9")
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_pairing_clipboard_code() {
        let peers = PeersConfig {
            device_id: "482-910-334".into(),
            passcode: "849201".into(),
            relay_token: "deadbeef".into(),
            device_name: "Office Windows PC".into(),
            listen_port: 47843,
            relay_url: "http://1.2.3.4:7845".into(),
            paired_peers: Vec::new(),
            incoming_secrets: Vec::new(),
        };

        let code = format_pairing_clipboard(&peers);
        assert!(code.starts_with("wakey://pair/482-910-334/849201?"));
        let parsed = parse_pairing_input(&code).unwrap();
        assert_eq!(parsed.device_id, "482-910-334");
        assert_eq!(parsed.passcode.as_deref(), Some("849201"));
        assert_eq!(parsed.name.as_deref(), Some("Office Windows PC"));
        assert_eq!(parsed.relay_url.as_deref(), Some("http://1.2.3.4:7845"));
    }

    #[test]
    fn parses_plain_id_and_passcode_formats() {
        let p1 = parse_pairing_input("482910334:849201").unwrap();
        assert_eq!(p1.device_id, "482-910-334");
        assert_eq!(p1.passcode.as_deref(), Some("849201"));

        let p2 = parse_pairing_input("719-204-881 123456").unwrap();
        assert_eq!(p2.device_id, "719-204-881");
        assert_eq!(p2.passcode.as_deref(), Some("123456"));
    }

    #[test]
    fn secret_authorized_rejects_passcode_and_accepts_incoming_secret() {
        let peers = PeersConfig {
            passcode: "849201".into(),
            incoming_secrets: vec!["sec_abc123".into()],
            ..PeersConfig::default()
        };
        assert!(!secret_authorized(&peers, "849201"));
        assert!(!secret_authorized(&peers, ""));
        assert!(secret_authorized(&peers, "sec_abc123"));
    }

    #[test]
    fn validates_lan_and_relay_urls_against_ssrf() {
        assert!(validate_lan_addr("192.168.1.5:47843").is_ok());
        assert!(validate_lan_addr("169.254.169.254:80").is_err());
        assert!(validate_lan_addr("127.0.0.1:80/evil").is_err());
        assert!(validate_lan_addr("evil.com\r\nX: 1").is_err());

        assert!(validate_relay_url("https://relay.example.com").is_ok());
        assert!(validate_relay_url("file:///etc/passwd").is_err());
        assert!(validate_relay_url("http://169.254.169.254/latest").is_err());
        assert!(validate_relay_url("http://metadata.google.internal").is_err());
    }
}
