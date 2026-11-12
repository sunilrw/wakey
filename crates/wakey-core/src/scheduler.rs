use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeDelta};

use crate::config::{ScheduleRule, WeekdayShort};

#[derive(Debug, Clone)]
pub struct ActiveSchedule {
    pub name: String,
    pub keep_display_on: bool,
    pub on_battery: bool,
    pub remaining: Duration,
    pub end_clock: String,
}

#[derive(Default)]
pub struct ScheduleState {
    skipped_until_date: HashMap<String, NaiveDate>,
}

impl ScheduleState {
    pub fn skip_today(&mut self, name: &str) {
        self.skipped_until_date
            .insert(name.to_string(), Local::now().date_naive());
    }

    pub fn is_skipped_on(&self, name: &str, date: NaiveDate) -> bool {
        self.skipped_until_date.get(name).copied() == Some(date)
    }
}

/// Evaluates all schedule rules at `now` and returns the highest-priority active match.
pub fn evaluate_schedules(
    rules: &[ScheduleRule],
    state: &ScheduleState,
    now: DateTime<Local>,
) -> Option<ActiveSchedule> {
    let today = now.date_naive();
    let current_wd = WeekdayShort::from_chrono(now.weekday());
    let current_time = now.time();

    let mut best: Option<ActiveSchedule> = None;

    for rule in rules.iter().filter(|r| r.enabled) {
        if state.is_skipped_on(&rule.name, today) {
            continue;
        }
        let Ok(start) = NaiveTime::parse_from_str(rule.start.trim(), "%H:%M") else {
            continue;
        };
        let Ok(end) = NaiveTime::parse_from_str(rule.end.trim(), "%H:%M") else {
            continue;
        };

        let in_window = if start <= end {
            rule.days.contains(&current_wd) && current_time >= start && current_time < end
        } else {
            // Overnight schedule (e.g. 22:00 - 02:00)
            if current_time >= start {
                rule.days.contains(&current_wd)
            } else if current_time < end {
                let yesterday_wd =
                    WeekdayShort::from_chrono((today - TimeDelta::days(1)).weekday());
                rule.days.contains(&yesterday_wd)
            } else {
                false
            }
        };

        if !in_window {
            continue;
        }

        let end_dt = if start <= end || current_time < end {
            today
                .and_time(end)
                .and_local_timezone(Local)
                .earliest()
                .unwrap_or(now)
        } else {
            (today + TimeDelta::days(1))
                .and_time(end)
                .and_local_timezone(Local)
                .earliest()
                .unwrap_or(now)
        };

        let rem_secs = (end_dt - now).num_seconds().max(1) as u64;
        let candidate = ActiveSchedule {
            name: rule.name.clone(),
            keep_display_on: rule.keep_display_on,
            on_battery: rule.on_battery,
            remaining: Duration::from_secs(rem_secs),
            end_clock: rule.end.clone(),
        };

        if best
            .as_ref()
            .map(|b| candidate.remaining > b.remaining)
            .unwrap_or(true)
        {
            best = Some(candidate);
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn matches_weekday_schedule_and_respects_skip() {
        let rules = vec![ScheduleRule {
            name: "Workday".into(),
            days: vec![WeekdayShort::Mon, WeekdayShort::Tue],
            start: "09:00".into(),
            end: "17:30".into(),
            keep_display_on: false,
            on_battery: false,
            enabled: true,
        }];

        // 2026-11-09 is a Monday
        let monday_10am = Local.with_ymd_and_hms(2026, 11, 9, 10, 0, 0).unwrap();
        let mut state = ScheduleState::default();
        let active = evaluate_schedules(&rules, &state, monday_10am).unwrap();
        assert_eq!(active.name, "Workday");
        assert_eq!(active.remaining, Duration::from_secs(7 * 3600 + 30 * 60));

        state
            .skipped_until_date
            .insert("Workday".into(), monday_10am.date_naive());
        assert!(evaluate_schedules(&rules, &state, monday_10am).is_none());
    }

    #[test]
    fn matches_overnight_schedule() {
        let rules = vec![ScheduleRule {
            name: "Night".into(),
            days: vec![WeekdayShort::Fri],
            start: "22:00".into(),
            end: "02:00".into(),
            keep_display_on: false,
            on_battery: true,
            enabled: true,
        }];

        // Saturday 01:00 AM (started Friday 22:00)
        let sat_1am = Local.with_ymd_and_hms(2026, 11, 14, 1, 0, 0).unwrap();
        let state = ScheduleState::default();
        let active = evaluate_schedules(&rules, &state, sat_1am).unwrap();
        assert_eq!(active.remaining, Duration::from_secs(3600));
    }
}
