use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Local, TimeDelta, Timelike};
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

pub fn append_record_with_retention(
    path: &Path,
    record: &SessionRecord,
    retention_days: u32,
    max_bytes: u64,
) -> std::io::Result<()> {
    append_record(path, record)?;
    prune_history(path, retention_days, max_bytes)
}

pub fn prune_history(path: &Path, retention_days: u32, max_bytes: u64) -> std::io::Result<()> {
    prune_history_at(path, retention_days, max_bytes, Local::now())
}

fn prune_history_at(
    path: &Path,
    retention_days: u32,
    max_bytes: u64,
    now: DateTime<Local>,
) -> std::io::Result<()> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let cutoff = (retention_days > 0).then(|| now - TimeDelta::days(i64::from(retention_days)));

    let mut kept: VecDeque<String> = VecDeque::new();
    let mut total_bytes: u64 = 0;

    for line in content.lines() {
        let Ok(rec) = serde_json::from_str::<SessionRecord>(line) else {
            continue;
        };
        if let Some(cutoff) = cutoff {
            if rec.timestamp < cutoff {
                continue;
            }
        }
        let line_bytes = (line.len() + 1) as u64;
        kept.push_back(line.to_string());
        total_bytes += line_bytes;
    }

    if max_bytes > 0 {
        while total_bytes > max_bytes {
            let Some(oldest) = kept.pop_front() else {
                break;
            };
            total_bytes = total_bytes.saturating_sub((oldest.len() + 1) as u64);
        }
    }

    let mut out = String::with_capacity(total_bytes as usize);
    for line in kept {
        out.push_str(&line);
        out.push('\n');
    }
    std::fs::write(path, out)
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

fn weekday_short_label(day: WeekdayShort) -> &'static str {
    match day {
        WeekdayShort::Mon => "Mon",
        WeekdayShort::Tue => "Tue",
        WeekdayShort::Wed => "Wed",
        WeekdayShort::Thu => "Thu",
        WeekdayShort::Fri => "Fri",
        WeekdayShort::Sat => "Sat",
        WeekdayShort::Sun => "Sun",
    }
}

fn weekday_key(day: WeekdayShort) -> &'static str {
    match day {
        WeekdayShort::Mon => "mon",
        WeekdayShort::Tue => "tue",
        WeekdayShort::Wed => "wed",
        WeekdayShort::Thu => "thu",
        WeekdayShort::Fri => "fri",
        WeekdayShort::Sat => "sat",
        WeekdayShort::Sun => "sun",
    }
}

fn is_weekend(day: WeekdayShort) -> bool {
    matches!(day, WeekdayShort::Sat | WeekdayShort::Sun)
}

fn format_duration_short(minutes: u32) -> String {
    if minutes >= 60 && minutes.is_multiple_of(60) {
        format!("{}h", minutes / 60)
    } else if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

fn build_suggestion(
    pattern_key: String,
    cadence_label: &str,
    hour: u16,
    items: &[&SessionRecord],
) -> HabitSuggestion {
    let mut durations: Vec<u32> = items.iter().map(|r| r.duration_minutes).collect();
    durations.sort_unstable();
    let median_dur = durations[durations.len() / 2].max(15);

    let display_votes = items.iter().filter(|r| r.keep_display).count();
    let keep_display = display_votes * 2 > items.len();

    let end_mins = (u32::from(hour) * 60 + median_dur) % (24 * 60);
    let suggested_until = format!("{:02}:{:02}", end_mins / 60, end_mins % 60);
    let dur_label = format_duration_short(median_dur);

    HabitSuggestion {
        pattern_key,
        description: format!("{cadence_label} ~{hour:02}:00 for {dur_label}"),
        suggested_until,
        duration_minutes: median_dur,
        occurrences: items.len(),
        keep_display,
    }
}

/// Analyzes `records` (e.g. full 90-day `history.jsonl`) and returns learned habit patterns
/// that meet `cfg.min_occurrences`, classified as Daily, Weekdays, Weekends, or Specific weekday.
pub fn detect_habits(records: &[SessionRecord], cfg: &HabitConfig) -> Vec<HabitSuggestion> {
    if !cfg.enabled || records.is_empty() {
        return Vec::new();
    }

    // Group by 60-minute start hour bucket
    let mut buckets: HashMap<u16, Vec<&SessionRecord>> = HashMap::new();
    for rec in records {
        let hour_bucket = (rec.start_minute_of_day / 60).min(23);
        buckets.entry(hour_bucket).or_default().push(rec);
    }

    let mut suggestions = Vec::new();
    for (hour, items) in buckets {
        let total = items.len();
        if total < cfg.min_occurrences {
            continue;
        }

        let mut by_day: HashMap<WeekdayShort, Vec<&SessionRecord>> = HashMap::new();
        let mut weekday_items: Vec<&SessionRecord> = Vec::new();
        let mut weekend_items: Vec<&SessionRecord> = Vec::new();

        for rec in &items {
            by_day.entry(rec.weekday).or_default().push(rec);
            if is_weekend(rec.weekday) {
                weekend_items.push(rec);
            } else {
                weekday_items.push(rec);
            }
        }

        let distinct_days = by_day.len();
        let distinct_weekdays = by_day.keys().filter(|d| !is_weekend(**d)).count();
        let distinct_weekends = by_day.keys().filter(|d| is_weekend(**d)).count();

        // 1. Daily: >=5 distinct days of the week, including weekend activity (>=10% of bucket).
        if distinct_days >= 5 && distinct_weekends >= 1 && weekend_items.len() * 10 >= total {
            suggestions.push(build_suggestion(
                format!("daily_{hour:02}"),
                "Daily",
                hour,
                &items,
            ));
            continue;
        }

        let mut covered_weekdays = false;
        let mut covered_weekends = false;

        // 2. Weekdays (Mon-Fri): multiple weekdays, not dominated (>=70%) by a single day.
        let max_wd_count = by_day
            .iter()
            .filter(|(d, _)| !is_weekend(**d))
            .map(|(_, v)| v.len())
            .max()
            .unwrap_or(0);
        let wd_single_day_dominated =
            max_wd_count >= cfg.min_occurrences && max_wd_count * 10 >= weekday_items.len() * 7;
        let two_independent_days = distinct_weekdays == 2
            && by_day
                .iter()
                .filter(|(d, v)| !is_weekend(**d) && v.len() >= cfg.min_occurrences)
                .count()
                == 2;

        if weekday_items.len() >= cfg.min_occurrences
            && distinct_weekdays >= 2
            && !wd_single_day_dominated
            && !two_independent_days
        {
            suggestions.push(build_suggestion(
                format!("weekdays_{hour:02}"),
                "Weekdays",
                hour,
                &weekday_items,
            ));
            covered_weekdays = true;
        }

        // 3. Weekends (Sat-Sun): both Sat and Sun represented, neither overwhelmingly dominant.
        let min_we_count = by_day
            .iter()
            .filter(|(d, _)| is_weekend(**d))
            .map(|(_, v)| v.len())
            .min()
            .unwrap_or(0);
        let we_balanced = min_we_count * 10 >= weekend_items.len() * 2
            || by_day
                .iter()
                .filter(|(d, v)| is_weekend(**d) && v.len() >= cfg.min_occurrences)
                .count()
                == 0;

        if weekend_items.len() >= cfg.min_occurrences && distinct_weekends == 2 && we_balanced {
            suggestions.push(build_suggestion(
                format!("weekends_{hour:02}"),
                "Weekends",
                hour,
                &weekend_items,
            ));
            covered_weekends = true;
        }

        // 4. Specific weekday (e.g. Every Wed) for any uncovered day meeting min_occurrences.
        for (day, day_items) in by_day {
            if is_weekend(day) && covered_weekends {
                continue;
            }
            if !is_weekend(day) && covered_weekdays {
                continue;
            }
            if day_items.len() >= cfg.min_occurrences {
                let label = format!("Every {}", weekday_short_label(day));
                let key = format!("{}_{hour:02}", weekday_key(day));
                suggestions.push(build_suggestion(key, &label, hour, &day_items));
            }
        }
    }

    suggestions.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.pattern_key.cmp(&b.pattern_key))
    });
    suggestions
}

fn suggestion_matches_day_hour(pattern_key: &str, today: WeekdayShort, hour: u16) -> bool {
    let Some((cadence, key_hour)) = pattern_key.rsplit_once('_') else {
        return false;
    };
    if key_hour.parse::<u16>().ok() != Some(hour) {
        return false;
    }
    match cadence {
        "daily" => true,
        "weekdays" => !is_weekend(today),
        "weekends" => is_weekend(today),
        other => other == weekday_key(today),
    }
}

/// Checks if any learned habit matches the current time `now` (hour bucket and cadence weekday).
pub fn matching_habit_now(
    records: &[SessionRecord],
    cfg: &HabitConfig,
    now: DateTime<Local>,
) -> Option<HabitSuggestion> {
    let today = WeekdayShort::from_chrono(now.weekday());
    let current_hour = now.hour() as u16;

    detect_habits(records, cfg)
        .into_iter()
        .find(|s| suggestion_matches_day_hour(&s.pattern_key, today, current_hour))
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
            ..HabitConfig::default()
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
        assert!(found.description.contains("Weekdays ~09:00 for 8h"));
    }

    fn make_session(
        now: DateTime<Local>,
        days_ago: i64,
        hour: u16,
        minute: u16,
        duration_minutes: u32,
    ) -> SessionRecord {
        let ts = now - TimeDelta::days(days_ago);
        SessionRecord {
            timestamp: ts,
            weekday: WeekdayShort::from_chrono(ts.weekday()),
            start_minute_of_day: hour * 60 + minute,
            duration_minutes,
            keep_display: false,
            source: "manual".into(),
        }
    }

    #[test]
    fn learns_daily_habit_across_multi_week_records() {
        let cfg = HabitConfig::default();
        // 2026-11-13 is Friday
        let base_fri = Local.with_ymd_and_hms(2026, 11, 13, 9, 15, 0).unwrap();
        let mut records = Vec::new();
        // 3 weeks of Mon..Sat (6 distinct days/week, includes Saturday)
        for week in 0..3 {
            for offset in 0..7 {
                let days_ago = week * 7 + offset;
                let ts = base_fri - TimeDelta::days(days_ago);
                if WeekdayShort::from_chrono(ts.weekday()) != WeekdayShort::Sun {
                    records.push(make_session(base_fri, days_ago, 9, 10, 120));
                }
            }
        }

        let habits = detect_habits(&records, &cfg);
        assert_eq!(habits.len(), 1);
        assert_eq!(habits[0].pattern_key, "daily_09");
        assert_eq!(habits[0].description, "Daily ~09:00 for 2h");

        // Matches both a weekday (Mon) and a weekend (Sun) at 09:xx
        let mon_09 = Local.with_ymd_and_hms(2026, 11, 16, 9, 30, 0).unwrap();
        let sun_09 = Local.with_ymd_and_hms(2026, 11, 15, 9, 5, 0).unwrap();
        let mon_14 = Local.with_ymd_and_hms(2026, 11, 16, 14, 0, 0).unwrap();
        assert_eq!(matching_habit_now(&records, &cfg, mon_09).unwrap().pattern_key, "daily_09");
        assert_eq!(matching_habit_now(&records, &cfg, sun_09).unwrap().pattern_key, "daily_09");
        assert!(matching_habit_now(&records, &cfg, mon_14).is_none());
    }

    #[test]
    fn learns_weekdays_habit_across_multi_week_records() {
        let cfg = HabitConfig::default();
        let base_fri = Local.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap();
        let mut records = Vec::new();
        // 4 weeks of Mon-Fri across ~30 days, plus 1 stray Saturday
        for days_ago in 0..28 {
            let ts = base_fri - TimeDelta::days(days_ago);
            if !is_weekend(WeekdayShort::from_chrono(ts.weekday())) {
                records.push(make_session(base_fri, days_ago, 9, 5, 120));
            }
        }
        // Single stray Saturday (1/21 < 10%) should not turn Weekdays into Daily
        records.push(make_session(base_fri, 6, 9, 0, 60));

        let habits = detect_habits(&records, &cfg);
        assert_eq!(habits.len(), 1);
        assert_eq!(habits[0].pattern_key, "weekdays_09");
        assert_eq!(habits[0].description, "Weekdays ~09:00 for 2h");

        let thu_09 = Local.with_ymd_and_hms(2026, 11, 12, 9, 45, 0).unwrap();
        let sat_09 = Local.with_ymd_and_hms(2026, 11, 14, 9, 15, 0).unwrap();
        assert_eq!(
            matching_habit_now(&records, &cfg, thu_09).unwrap().pattern_key,
            "weekdays_09"
        );
        assert!(matching_habit_now(&records, &cfg, sat_09).is_none());
    }

    #[test]
    fn learns_weekends_habit_across_multi_week_records() {
        let cfg = HabitConfig::default();
        let base_sun = Local.with_ymd_and_hms(2026, 11, 15, 10, 0, 0).unwrap();
        let mut records = Vec::new();
        // 4 weekends (Sat + Sun) over 28 days
        for week in 0..4 {
            records.push(make_session(base_sun, week * 7, 10, 15, 180)); // Sun
            records.push(make_session(base_sun, week * 7 + 1, 10, 20, 180)); // Sat
        }

        let habits = detect_habits(&records, &cfg);
        assert_eq!(habits.len(), 1);
        assert_eq!(habits[0].pattern_key, "weekends_10");
        assert_eq!(habits[0].description, "Weekends ~10:00 for 3h");

        let sat_10 = Local.with_ymd_and_hms(2026, 11, 14, 10, 30, 0).unwrap();
        let sun_10 = Local.with_ymd_and_hms(2026, 11, 15, 10, 5, 0).unwrap();
        let mon_10 = Local.with_ymd_and_hms(2026, 11, 16, 10, 10, 0).unwrap();
        assert_eq!(
            matching_habit_now(&records, &cfg, sat_10).unwrap().pattern_key,
            "weekends_10"
        );
        assert_eq!(
            matching_habit_now(&records, &cfg, sun_10).unwrap().pattern_key,
            "weekends_10"
        );
        assert!(matching_habit_now(&records, &cfg, mon_10).is_none());
    }

    #[test]
    fn learns_specific_weekday_habit_across_90_day_records() {
        let cfg = HabitConfig::default();
        // 2026-11-11 is Wednesday
        let base_wed = Local.with_ymd_and_hms(2026, 11, 11, 14, 0, 0).unwrap();
        let mut records = Vec::new();
        // Every Wednesday for 12 weeks (84 days, full 90-day window)
        for week in 0..12 {
            records.push(make_session(base_wed, week * 7, 14, 10, 60));
        }

        let habits = detect_habits(&records, &cfg);
        assert_eq!(habits.len(), 1);
        assert_eq!(habits[0].pattern_key, "wed_14");
        assert_eq!(habits[0].occurrences, 12);
        assert_eq!(habits[0].description, "Every Wed ~14:00 for 1h");

        let wed_14 = Local.with_ymd_and_hms(2026, 11, 18, 14, 25, 0).unwrap();
        let thu_14 = Local.with_ymd_and_hms(2026, 11, 19, 14, 25, 0).unwrap();
        assert_eq!(
            matching_habit_now(&records, &cfg, wed_14).unwrap().pattern_key,
            "wed_14"
        );
        assert!(matching_habit_now(&records, &cfg, thu_14).is_none());
    }

    #[test]
    fn prunes_old_and_oversized_history() {
        let dir = std::env::temp_dir().join(format!("wakey-habits-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.jsonl");
        let _ = std::fs::remove_file(&path);

        let now = Local.with_ymd_and_hms(2026, 11, 13, 10, 0, 0).unwrap();
        let make_rec = |days_ago: i64| SessionRecord {
            timestamp: now - TimeDelta::days(days_ago),
            weekday: WeekdayShort::Mon,
            start_minute_of_day: 9 * 60,
            duration_minutes: 120,
            keep_display: false,
            source: "manual".into(),
        };

        // Older than 90 days -> should be dropped by retention_days = 90
        append_record(&path, &make_rec(120)).unwrap();
        // Within 90 days -> candidates kept by age, then oldest dropped by size cap
        append_record(&path, &make_rec(30)).unwrap();
        append_record(&path, &make_rec(5)).unwrap();
        append_record(&path, &make_rec(1)).unwrap();

        let single_line_len = serde_json::to_string(&make_rec(1)).unwrap().len() as u64 + 1;
        prune_history_at(&path, 90, single_line_len * 2 + 5, now).unwrap();

        let kept = load_records(&path);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].timestamp, now - TimeDelta::days(5));
        assert_eq!(kept[1].timestamp, now - TimeDelta::days(1));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
