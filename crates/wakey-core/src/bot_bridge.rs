use std::io::Read;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::config::{constant_time_eq, BotConfig};

const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotEventPayload {
    pub event: String,
    pub title: String,
    pub message: String,
    pub actions: Vec<String>,
    pub timestamp: DateTime<Local>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum ControlCommand {
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
pub struct BotCommandRequest {
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotCommandResponse {
    pub ok: bool,
    pub message: String,
}

/// Parses free-form text from a Grok bot or chat command (e.g. `"extend 1h"`, `"awake 2h"`, `"stop"`, `"status"`).
pub fn parse_bot_text_command(raw: &str) -> Result<ControlCommand, String> {
    let trimmed = raw.trim();
    let lower = trimmed.to_lowercase();
    let parts: Vec<&str> = lower.split_whitespace().collect();

    match parts.as_slice() {
        ["status"] | ["?"] => Ok(ControlCommand::Status),
        ["stop"] | ["sleep"] | ["off"] => Ok(ControlCommand::Stop),
        ["pause"] => Ok(ControlCommand::Pause),
        ["resume"] => Ok(ControlCommand::Resume),
        ["yes"] | ["extend"] => Ok(ControlCommand::Extend {
            duration: "30m".into(),
        }),
        ["extend", dur] => Ok(ControlCommand::Extend {
            duration: (*dur).to_string(),
        }),
        ["awake", dur] | ["wake", dur] => Ok(ControlCommand::AwakeFor {
            duration: (*dur).to_string(),
            display: false,
        }),
        ["until", clock] => Ok(ControlCommand::AwakeUntil {
            clock: (*clock).to_string(),
            display: false,
        }),
        ["skip", name @ ..] if !name.is_empty() => Ok(ControlCommand::SkipSchedule {
            name: name.join(" "),
        }),
        _ => Err(format!(
            "unknown command '{trimmed}'. Try: status, awake 2h, until 22:00, extend 30m, stop, pause, resume"
        )),
    }
}

/// Sends an outbound webhook event to the configured Grok bot endpoint (non-blocking thread).
pub fn send_bot_event_async(cfg: &BotConfig, payload: BotEventPayload) {
    if !cfg.enabled || cfg.webhook_url.trim().is_empty() {
        return;
    }
    let url = cfg.webhook_url.clone();
    let secret = cfg.webhook_secret.clone();

    thread::spawn(move || {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build();
        let mut req = agent.post(&url).set("Content-Type", "application/json");
        if !secret.is_empty() {
            req = req
                .set("X-Wakey-Secret", &secret)
                .set("Authorization", &format!("Bearer {secret}"));
        }
        let _ = req.send_json(serde_json::to_value(&payload).unwrap_or_default());
    });
}

/// Starts the authenticated local HTTP bridge for inbound Grok bot commands (`POST /command`, `GET /status`).
pub fn spawn_http_bridge(
    cfg: BotConfig,
    cmd_tx: Sender<(ControlCommand, Sender<String>)>,
) -> std::io::Result<()> {
    if !cfg.enabled {
        return Ok(());
    }
    let server = tiny_http::Server::http(&cfg.listen_addr)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    let auth_token = cfg.auth_token.trim().to_string();

    thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let expected = format!("Bearer {auth_token}");
            let authorized = !auth_token.is_empty()
                && request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| constant_time_eq(h.value.as_str(), &expected))
                    .unwrap_or(false);
            if !authorized {
                let msg = if auth_token.is_empty() {
                    r#"{"ok":false,"message":"bot bridge auth_token is not configured"}"#
                } else {
                    r#"{"ok":false,"message":"unauthorized"}"#
                };
                let resp = tiny_http::Response::from_string(msg).with_status_code(401);
                let _ = request.respond(resp);
                continue;
            }

            let url = request.url().to_string();
            let method = request.method().as_str().to_uppercase();

            if method == "GET" && url == "/status" {
                let (reply_tx, reply_rx) = std::sync::mpsc::channel();
                let _ = cmd_tx.send((ControlCommand::Status, reply_tx));
                let body = reply_rx.recv_timeout(Duration::from_secs(2)).unwrap_or_else(|_| {
                    r#"{"ok":false,"message":"daemon timeout"}"#.into()
                });
                let header = tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();
                let _ = request.respond(tiny_http::Response::from_string(body).with_header(header));
                continue;
            }

            if method == "POST" && url == "/command" {
                if request
                    .body_length()
                    .map(|len| len > MAX_HTTP_BODY_BYTES)
                    .unwrap_or(false)
                {
                    let _ = request.respond(
                        tiny_http::Response::from_string(
                            r#"{"ok":false,"message":"payload too large"}"#,
                        )
                        .with_status_code(413),
                    );
                    continue;
                }
                let mut body = String::new();
                if request
                    .as_reader()
                    .take((MAX_HTTP_BODY_BYTES + 1) as u64)
                    .read_to_string(&mut body)
                    .is_err()
                {
                    let _ = request.respond(
                        tiny_http::Response::from_string(
                            r#"{"ok":false,"message":"failed to read body"}"#,
                        )
                        .with_status_code(400),
                    );
                    continue;
                }
                if body.len() > MAX_HTTP_BODY_BYTES {
                    let _ = request.respond(
                        tiny_http::Response::from_string(
                            r#"{"ok":false,"message":"payload too large"}"#,
                        )
                        .with_status_code(413),
                    );
                    continue;
                }
                let parsed_req: Result<BotCommandRequest, _> = serde_json::from_str(&body);
                let reply_json = match parsed_req {
                    Ok(req) => match parse_bot_text_command(&req.command) {
                        Ok(cmd) => {
                            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
                            let _ = cmd_tx.send((cmd, reply_tx));
                            let msg = reply_rx
                                .recv_timeout(Duration::from_secs(2))
                                .unwrap_or_else(|_| "timeout".into());
                            serde_json::to_string(&BotCommandResponse {
                                ok: true,
                                message: msg,
                            })
                            .unwrap()
                        }
                        Err(e) => serde_json::to_string(&BotCommandResponse {
                            ok: false,
                            message: e,
                        })
                        .unwrap(),
                    },
                    Err(e) => serde_json::to_string(&BotCommandResponse {
                        ok: false,
                        message: format!("invalid JSON: {e}"),
                    })
                    .unwrap(),
                };
                let header = tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();
                let _ = request.respond(tiny_http::Response::from_string(reply_json).with_header(header));
                continue;
            }

            let _ = request.respond(
                tiny_http::Response::from_string(r#"{"ok":false,"message":"not found"}"#)
                    .with_status_code(404),
            );
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bot_commands() {
        assert!(matches!(
            parse_bot_text_command("extend 1h").unwrap(),
            ControlCommand::Extend { duration } if duration == "1h"
        ));
        assert!(matches!(
            parse_bot_text_command("awake 30m").unwrap(),
            ControlCommand::AwakeFor { duration, .. } if duration == "30m"
        ));
        assert!(matches!(
            parse_bot_text_command("until 22:30").unwrap(),
            ControlCommand::AwakeUntil { clock, .. } if clock == "22:30"
        ));
        assert!(matches!(
            parse_bot_text_command("stop").unwrap(),
            ControlCommand::Stop
        ));
        assert!(parse_bot_text_command("nonsense").is_err());
    }
}
