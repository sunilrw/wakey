use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::config::{AgentDetectionMode, AgentWatchConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedAgent {
    pub name: String,
    pub pid: u32,
    pub process_name: String,
    pub cpu_percent: f32,
    pub active_children: usize,
    pub is_actively_working: bool,
    pub mode: AgentDetectionMode,
}

pub struct AgentWatcher {
    sys: System,
    last_active_at: Option<Instant>,
    last_active_label: Option<String>,
}

impl AgentWatcher {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        Self {
            sys,
            last_active_at: None,
            last_active_label: None,
        }
    }

    /// Scans running processes and returns all matching agents/watched rules.
    pub fn scan(&mut self, cfg: &AgentWatchConfig) -> Vec<DetectedAgent> {
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );

        // Build parent -> children map so we can detect agents spawning builds/tests.
        let mut children_map: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for (pid, proc_) in self.sys.processes() {
            if let Some(parent) = proc_.parent() {
                children_map.entry(parent).or_default().push(*pid);
            }
        }

        let mut results = Vec::new();
        let mut seen_pids = HashSet::new();

        for (pid, proc_) in self.sys.processes() {
            let proc_name = proc_.name().to_string_lossy().to_lowercase();
            let cmdline = proc_
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");

            // Skip wakey itself and noisy Electron/runtime helper processes.
            if proc_name.starts_with("wakey")
                || proc_name.contains("helper")
                || proc_name == "disclaimer"
            {
                continue;
            }

            let is_script_runner = matches!(
                proc_name.as_str(),
                "node" | "node.exe" | "python" | "python3" | "python.exe" | "bun" | "deno" | "uv"
            );

            let mut matched: Option<(String, AgentDetectionMode)> = None;

            for preset in cfg.presets.iter().filter(|p| p.enabled) {
                let name_hit = preset
                    .process_names
                    .iter()
                    .any(|p| proc_name == p.to_lowercase());
                let cmd_hit = is_script_runner
                    && !preset.cmdline_keywords.is_empty()
                    && preset
                        .cmdline_keywords
                        .iter()
                        .any(|kw| cmdline.contains(&kw.to_lowercase()));
                if name_hit || cmd_hit {
                    matched = Some((preset.name.clone(), preset.mode));
                    break;
                }
            }

            if matched.is_none() {
                for rule in cfg.custom_rules.iter().filter(|r| r.enabled) {
                    let pat = rule.pattern.to_lowercase();
                    if proc_name.contains(&pat) || cmdline.contains(&pat) {
                        matched = Some((rule.name.clone(), rule.mode));
                        break;
                    }
                }
            }

            let Some((label, mode)) = matched else {
                continue;
            };

            if !seen_pids.insert(pid.as_u32()) {
                continue;
            }

            let cpu = proc_.cpu_usage();
            let child_pids = children_map.get(pid).map(|v| v.as_slice()).unwrap_or(&[]);
            let active_children = child_pids
                .iter()
                .filter(|cpid| {
                    self.sys
                        .process(**cpid)
                        .map(|cp| cp.cpu_usage() >= 0.2)
                        .unwrap_or(false)
                })
                .count();

            let is_actively_working = match mode {
                AgentDetectionMode::ProcessExists => true,
                AgentDetectionMode::ActivelyWorking => {
                    cpu >= cfg.cpu_threshold_percent || active_children > 0
                }
            };

            results.push(DetectedAgent {
                name: label,
                pid: pid.as_u32(),
                process_name: proc_.name().to_string_lossy().to_string(),
                cpu_percent: cpu,
                active_children,
                is_actively_working,
                mode,
            });
        }

        results
    }

    /// Evaluates whether Agent Watch should currently keep the machine awake (including grace period).
    pub fn evaluate(&mut self, cfg: &AgentWatchConfig) -> Option<String> {
        if !cfg.enabled {
            return None;
        }
        let detected = self.scan(cfg);
        if let Some(active) = detected.iter().find(|d| d.is_actively_working) {
            self.last_active_at = Some(Instant::now());
            self.last_active_label = Some(active.name.clone());
            return Some(format!("Agent active: {}", active.name));
        }

        if let (Some(last), Some(label)) = (self.last_active_at, self.last_active_label.as_ref()) {
            let grace = Duration::from_secs(u64::from(cfg.grace_period_minutes) * 60);
            if last.elapsed() < grace {
                let rem = (grace - last.elapsed()).as_secs() / 60 + 1;
                return Some(format!("Agent cooldown ({label}, ~{rem}m left)"));
            }
        }

        None
    }
}

impl Default for AgentWatcher {
    fn default() -> Self {
        Self::new()
    }
}
