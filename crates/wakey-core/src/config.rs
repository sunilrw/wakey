use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config at {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("failed to parse config TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize config TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("failed to write config at {0}: {1}")]
    Write(PathBuf, std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeekdayShort {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl WeekdayShort {
    pub fn from_chrono(wd: chrono::Weekday) -> Self {
        match wd {
            chrono::Weekday::Mon => Self::Mon,
            chrono::Weekday::Tue => Self::Tue,
            chrono::Weekday::Wed => Self::Wed,
            chrono::Weekday::Thu => Self::Thu,
            chrono::Weekday::Fri => Self::Fri,
            chrono::Weekday::Sat => Self::Sat,
            chrono::Weekday::Sun => Self::Sun,
        }
    }

    pub fn parse_list(spec: &str) -> Vec<Self> {
        let lower = spec.trim().to_lowercase();
        if lower == "daily" || lower == "everyday" || lower == "all" {
            return vec![
                Self::Mon,
                Self::Tue,
                Self::Wed,
                Self::Thu,
                Self::Fri,
                Self::Sat,
                Self::Sun,
            ];
        }
        if lower == "weekdays" || lower == "mon-fri" {
            return vec![Self::Mon, Self::Tue, Self::Wed, Self::Thu, Self::Fri];
        }
        if lower == "weekends" || lower == "sat-sun" {
            return vec![Self::Sat, Self::Sun];
        }
        lower
            .split(',')
            .filter_map(|part| match part.trim() {
                "mon" | "monday" => Some(Self::Mon),
                "tue" | "tuesday" => Some(Self::Tue),
                "wed" | "wednesday" => Some(Self::Wed),
                "thu" | "thursday" => Some(Self::Thu),
                "fri" | "friday" => Some(Self::Fri),
                "sat" | "saturday" => Some(Self::Sat),
                "sun" | "sunday" => Some(Self::Sun),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDetectionMode {
    /// Only stay awake when the agent is actively using CPU or spawning child processes.
    ActivelyWorking,
    /// Stay awake whenever the process exists.
    ProcessExists,
}

impl Default for AgentDetectionMode {
    fn default() -> Self {
        Self::ActivelyWorking
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub auto_start: bool,
    pub start_minimized: bool,
    pub allow_screen_off_default: bool,
    pub screen_off_minutes: u32,
    pub pause_on_battery: bool,
    pub min_battery_percent: u8,
    pub pause_on_manual_lock: bool,
    pub warn_before_minutes: u32,
    pub global_hotkey: String,
    pub theme: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            start_minimized: true,
            allow_screen_off_default: true,
            screen_off_minutes: 5,
            pause_on_battery: true,
            min_battery_percent: 20,
            pause_on_manual_lock: false,
            warn_before_minutes: 5,
            global_hotkey: "Ctrl+Alt+W".into(),
            theme: "system".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HabitConfig {
    pub enabled: bool,
    pub min_occurrences: usize,
    pub prompt_cooldown_minutes: u32,
    pub retention_days: u32,
    pub max_history_bytes: u64,
}

impl Default for HabitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_occurrences: 3,
            prompt_cooldown_minutes: 120,
            retention_days: 90,
            max_history_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPreset {
    pub id: String,
    pub name: String,
    pub process_names: Vec<String>,
    pub cmdline_keywords: Vec<String>,
    pub mode: AgentDetectionMode,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomWatchRule {
    pub name: String,
    pub pattern: String,
    #[serde(default)]
    pub mode: AgentDetectionMode,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentWatchConfig {
    pub enabled: bool,
    pub keep_display_on: bool,
    pub on_battery: bool,
    pub grace_period_minutes: u32,
    pub cpu_threshold_percent: f32,
    pub presets: Vec<AgentPreset>,
    pub custom_rules: Vec<CustomWatchRule>,
}

impl Default for AgentWatchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_display_on: false,
            on_battery: false,
            grace_period_minutes: 5,
            cpu_threshold_percent: 1.5,
            presets: default_agent_presets(),
            custom_rules: Vec::new(),
        }
    }
}

pub fn default_agent_presets() -> Vec<AgentPreset> {
    vec![
        AgentPreset {
            id: "claude".into(),
            name: "Claude Code".into(),
            process_names: vec!["claude".into(), "claude.exe".into()],
            cmdline_keywords: vec!["@anthropic-ai/claude-code".into(), "claude-code".into()],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "codex".into(),
            name: "OpenAI Codex CLI".into(),
            process_names: vec!["codex".into(), "codex.exe".into()],
            cmdline_keywords: vec!["@openai/codex".into()],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "gemini".into(),
            name: "Gemini CLI".into(),
            process_names: vec!["gemini".into(), "gemini.exe".into()],
            cmdline_keywords: vec!["@google/gemini-cli".into(), "gemini-cli".into()],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "opencode".into(),
            name: "OpenCode".into(),
            process_names: vec!["opencode".into(), "opencode.exe".into()],
            cmdline_keywords: vec!["opencode".into()],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "aider".into(),
            name: "Aider".into(),
            process_names: vec!["aider".into(), "aider.exe".into()],
            cmdline_keywords: vec!["aider".into()],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "goose".into(),
            name: "Goose".into(),
            process_names: vec!["goose".into(), "goose.exe".into()],
            cmdline_keywords: vec![],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "antigravity".into(),
            name: "Antigravity".into(),
            process_names: vec!["antigravity".into(), "antigravity.exe".into()],
            cmdline_keywords: vec!["antigravity".into()],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "cursor".into(),
            name: "Cursor".into(),
            process_names: vec!["cursor".into(), "cursor.exe".into()],
            cmdline_keywords: vec![],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "windsurf".into(),
            name: "Windsurf".into(),
            process_names: vec!["windsurf".into(), "windsurf.exe".into()],
            cmdline_keywords: vec![],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "ollama".into(),
            name: "Ollama".into(),
            process_names: vec!["ollama".into(), "ollama.exe".into(), "ollama_llama_server".into()],
            cmdline_keywords: vec![],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
        AgentPreset {
            id: "lmstudio".into(),
            name: "LM Studio / llama-server".into(),
            process_names: vec!["llama-server".into(), "llama-server.exe".into(), "lmstudio".into()],
            cmdline_keywords: vec!["lm-studio".into()],
            mode: AgentDetectionMode::ActivelyWorking,
            enabled: true,
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BotConfig {
    pub enabled: bool,
    pub webhook_url: String,
    pub webhook_secret: String,
    pub listen_addr: String,
    pub auth_token: String,
    pub notify_on_start: bool,
    pub notify_on_agent: bool,
    pub notify_on_pre_expiry: bool,
    pub notify_on_habit: bool,
    pub notify_on_end: bool,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            webhook_url: String::new(),
            webhook_secret: String::new(),
            listen_addr: "127.0.0.1:7842".into(),
            auth_token: String::new(),
            notify_on_start: true,
            notify_on_agent: true,
            notify_on_pre_expiry: true,
            notify_on_habit: true,
            notify_on_end: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRule {
    pub name: String,
    pub days: Vec<WeekdayShort>,
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub keep_display_on: bool,
    #[serde(default)]
    pub on_battery: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedPeer {
    pub device_id: String,
    pub name: String,
    pub shared_secret: String,
    #[serde(default)]
    pub direct_addr: Option<String>,
    #[serde(default)]
    pub mac_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PeersConfig {
    pub device_id: String,
    pub passcode: String,
    pub relay_token: String,
    pub device_name: String,
    pub listen_port: u16,
    pub relay_url: String,
    pub paired_peers: Vec<PairedPeer>,
    pub incoming_secrets: Vec<String>,
}

impl Default for PeersConfig {
    fn default() -> Self {
        Self {
            device_id: String::new(),
            passcode: String::new(),
            relay_token: String::new(),
            device_name: default_device_name(),
            listen_port: 47843,
            relay_url: String::new(),
            paired_peers: Vec::new(),
            incoming_secrets: Vec::new(),
        }
    }
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "This Mac".into()
            } else if cfg!(target_os = "windows") {
                "Windows PC".into()
            } else {
                "Linux PC".into()
            }
        })
}

static CONFIG_SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn csprng_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).expect("OS CSPRNG unavailable");
    buf
}

fn csprng_u64() -> u64 {
    u64::from_le_bytes(csprng_bytes::<8>())
}

/// Constant-time string equality comparison for secrets, tokens, and passcodes.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let max_len = a_bytes.len().max(b_bytes.len());
    let mut diff: u8 = if a_bytes.len() == b_bytes.len() { 0 } else { 1 };
    for i in 0..max_len {
        let x = a_bytes.get(i).copied().unwrap_or(0);
        let y = b_bytes.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0
}

pub fn generate_device_id() -> String {
    let n = 100_000_000 + (csprng_u64() % 900_000_000);
    let s = format!("{n:09}");
    format!("{}-{}-{}", &s[0..3], &s[3..6], &s[6..9])
}

pub fn generate_passcode() -> String {
    let n = 100_000 + (csprng_u64() % 900_000);
    format!("{n:06}")
}

pub fn generate_shared_secret() -> String {
    use std::fmt::Write;
    let bytes = csprng_bytes::<16>();
    let mut out = String::with_capacity(32);
    for b in bytes {
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub habits: HabitConfig,
    pub agent_watch: AgentWatchConfig,
    pub bot: BotConfig,
    pub peers: PeersConfig,
    pub schedules: Vec<ScheduleRule>,
}

impl Config {
    /// Returns the active config file path (portable `wakey.toml` next to binary if present,
    /// otherwise OS config directory).
    pub fn default_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let portable = dir.join("wakey.toml");
                if portable.exists() {
                    return portable;
                }
            }
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wakey")
            .join("config.toml")
    }

    pub fn data_dir() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                if dir.join("wakey.toml").exists() {
                    return dir.to_path_buf();
                }
            }
        }
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wakey")
    }

    pub fn ensure_peer_identity(&mut self) -> bool {
        let mut changed = false;
        if self.peers.device_id.trim().is_empty() {
            self.peers.device_id = generate_device_id();
            changed = true;
        }
        if self.peers.passcode.trim().is_empty() {
            self.peers.passcode = generate_passcode();
            changed = true;
        }
        if self.peers.relay_token.trim().is_empty() {
            self.peers.relay_token = generate_shared_secret();
            changed = true;
        }
        if self.peers.device_name.trim().is_empty() {
            self.peers.device_name = default_device_name();
            changed = true;
        }
        changed
    }

    pub fn load_or_default() -> Result<Self, ConfigError> {
        let path = Self::default_path();
        let mut cfg = Self::load_from(&path)?;
        if cfg.ensure_peer_identity() {
            let _ = cfg.save_to(&path);
        }
        Ok(cfg)
    }

    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::Read(path.to_path_buf(), e))?;
        let parsed: Self = toml::from_str(&raw)?;
        Ok(parsed)
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        self.save_to(&Self::default_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        use std::io::Write;
        let _guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ConfigError::Write(path.to_path_buf(), e))?;
        }
        let serialized = toml::to_string_pretty(self)?;
        let tmp_name = format!(
            ".{}.tmp.{}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("config.toml"),
            std::process::id()
        );
        let tmp_path = path.with_file_name(tmp_name);

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }

        let mut file = opts
            .open(&tmp_path)
            .map_err(|e| ConfigError::Write(tmp_path.clone(), e))?;
        file.write_all(serialized.as_bytes())
            .map_err(|e| ConfigError::Write(tmp_path.clone(), e))?;
        file.sync_all()
            .map_err(|e| ConfigError::Write(tmp_path.clone(), e))?;
        drop(file);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600));
        }

        std::fs::rename(&tmp_path, path)
            .map_err(|e| ConfigError::Write(path.to_path_buf(), e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_default_config() {
        let dir = std::env::temp_dir().join(format!("wakey_cfg_test_{}", std::process::id()));
        let path = dir.join("config.toml");
        let cfg = Config::default();
        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.general.min_battery_percent, 20);
        assert!(!loaded.agent_watch.presets.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_weekday_lists() {
        assert_eq!(WeekdayShort::parse_list("mon-fri").len(), 5);
        assert_eq!(WeekdayShort::parse_list("daily").len(), 7);
        assert_eq!(WeekdayShort::parse_list("sat,sun").len(), 2);
    }

    #[test]
    fn csprng_and_constant_time_eq_work() {
        let s1 = generate_shared_secret();
        let s2 = generate_shared_secret();
        assert_eq!(s1.len(), 32);
        assert_ne!(s1, s2);
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc123", "abc12"));
    }

    #[cfg(unix)]
    #[test]
    fn config_saved_with_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("wakey_perm_test_{}", std::process::id()));
        let path = dir.join("config.toml");
        let cfg = Config::default();
        cfg.save_to(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }
}
