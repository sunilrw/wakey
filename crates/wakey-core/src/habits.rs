use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};

use crate::config::{Config, HabitConfig, WeekdayShort};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub timestamp: DateTime<Local>,
    pub weekday: WeekdayShort,
    pub start_minute_of_day: u16,
    pub duration_minutes: u32,
    pub keep_display: bool,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HabitSuggestion {
    pub pattern_key: String,
    pub description: String,
    pub suggested_until: String,
    pub duration_minutes: u32,
    pub occurrences: usize,
    pub keep_display: bool,
}

pub fn default_history_path() -> PathBuf {
    Config::data_dir().join("history.jsonl")
}

pub fn append_record(path: &Path, record: &SessionRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let line = serde_json::to_string(record).unwrap_or_default();
    writeln!(file, "{line}")?;
    Ok(())
}

pub fn load_records(path: &Path) -> Vec<SessionRecord> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<SessionRecord>(line).ok())
        .collect()
}

/// Analyzes `records` and returns learned habit patterns that meet `cfg.min_occurrences`.
pub fn detect_habits(records: &[SessionRecord], cfg: &HabitConfig) -> Vec<HabitSuggestion> {
    if !cfg.enabled || records.is_empty() {
        return Vec::new();
    }

    // Group by (is_weekday, 60-minute start hour bucket)
    let mut buckets: HashMap<(bool, u16), Vec<&SessionRecord>> = HashMap::new();
    for rec in records {
        let is_weekday = !matches!(rec.weekday, WeekdayShort::Sat | WeekdayShort::Sun);
        let hour_bucket = rec.start_minute_of_day / 60;
        buckets.entry((is_weekday, hour_bucket)).or_default().push(rec);
    }

    let mut suggestions = Vec::new();
    for ((is_weekday, hour), items) in buckets {
        if items.len() < cfg.min_occurrences {
            continue;
        }
        let mut durations: Vec<u32> = items.iter().map(|r| r.duration_minutes).collect();
        durations.sort_unstable();
        let median_dur = durations[durations.len() / 2].max(15);

        let display_votes = items.iter().filter(|r| r.keep_display).count();
        let keep_display = display_votes * 2 > items.len();

        let end_mins = (u32::from(hour) * 60 + median_dur) % (24 * 60);
        let suggested_until = format!("{:02}:{:02}", end_mins / 60, end_mins % 60);
        let day_label = if is_weekday { "weekdays" } else { "weekends" };

        suggestions.push(HabitSuggestion {
            pattern_key: format!("{day_label}_{hour:02}"),
            description: format!(
                "You usually keep your PC awake around {hour:02}:00 on {day_label} (~{}m, until {suggested_until})",
                median_dur
            ),
            suggested_until,
            duration_minutes: median_dur,
            occurrences: items.len(),
            keep_display,
        });
    }

    suggestions.sort_by(|a, b| b.occurrences.cmp(&a.occurrences));
    suggestions
}

/// Checks if any learned habit matches the current time `now` (within ±30m of the bucket hour).
pub fn matching_habit_now(
    records: &[SessionRecord],
    cfg: &HabitConfig,
    now: DateTime<Local>,
) -> Option<HabitSuggestion> {
    let is_weekday = !matches!(
        WeekdayShort::from_chrono(now.weekday()),
        WeekdayShort::Sat | WeekdayShort::Sun
    );
    let current_hour = now.hour() as u16;
    let day_label = if is_weekday { "weekdays" } else { "weekends" };
    let target_key = format!("{day_label}_{current_hour:02}");

    detect_habits(records, cfg)
        .into_iter()
        .find(|s| s.pattern_key == target_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn detects_repeated_weekday_habit() {
        let cfg = HabitConfig {
            enabled: true,
            min_occurrences: 3,
            prompt_cooldown_minutes: 120,
        };
        let ts = Local.with_ymd_and_hms(2026, 11, 9, 9, 10, 0).unwrap();
        let records = vec![
            SessionRecord {
                timestamp: ts,
                weekday: WeekdayShort::Mon,
                start_minute_of_day: 9 * 60 + 10,
                duration_minutes: 480,
                keep_display: false,
                source: "manual".into(),
            },
            SessionRecord {
                timestamp: ts,
                weekday: WeekdayShort::Tue,
                start_minute_of_day: 9 * 60 + 5,
                duration_minutes: 480,
                keep_display: false,
                source: "manual".into(),
            },
            SessionRecord {
                timestamp: ts,
                weekday: WeekdayShort::Wed,
                start_minute_of_day: 9 * 60 + 20,
                duration_minutes: 420,
                keep_display: false,
                source: "manual".into(),
            },
        ];

        let found = matching_habit_now(&records, &cfg, ts).unwrap();
        assert_eq!(found.occurrences, 3);
        assert_eq!(found.suggested_until, "17:00");
    }
}
