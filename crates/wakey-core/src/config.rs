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
}

impl Default for HabitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_occurrences: 3,
            prompt_cooldown_minutes: 120,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub habits: HabitConfig,
    pub agent_watch: AgentWatchConfig,
    pub bot: BotConfig,
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

    pub fn load_or_default() -> Result<Self, ConfigError> {
        Self::load_from(&Self::default_path())
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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ConfigError::Write(path.to_path_buf(), e))?;
        }
        let serialized = toml::to_string_pretty(self)?;
        std::fs::write(path, serialized).map_err(|e| ConfigError::Write(path.to_path_buf(), e))?;
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
}
