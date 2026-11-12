use std::time::Duration;

use chrono::{Local, NaiveTime, TimeDelta};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TimeParseError {
    #[error("invalid duration '{0}' (expected e.g. '30m', '2h', '1h30m', or seconds like '300')")]
    InvalidDuration(String),
    #[error("invalid time '{0}' (expected HH:MM or HH:MM:SS in 24-hour format)")]
    InvalidClockTime(String),
}

/// Parses either a plain integer (seconds, matching `caffeinate -t`) or a human duration (`30m`, `1h30m`).
pub fn parse_duration(input: &str) -> Result<Duration, TimeParseError> {
    let trimmed = input.trim();
    if let Ok(secs) = trimmed.parse::<u64>() {
        return Ok(Duration::from_secs(secs));
    }
    humantime::parse_duration(trimmed).map_err(|_| TimeParseError::InvalidDuration(input.to_string()))
}

/// Parses a local 24-hour clock time (`HH:MM` or `HH:MM:SS`) and returns the duration from now until
/// that time today (or tomorrow if the time has already passed today).
pub fn duration_until_clock(input: &str) -> Result<Duration, TimeParseError> {
    let trimmed = input.trim();
    let target_time = NaiveTime::parse_from_str(trimmed, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(trimmed, "%H:%M:%S"))
        .map_err(|_| TimeParseError::InvalidClockTime(input.to_string()))?;

    let now = Local::now();
    let today_naive = now.date_naive().and_time(target_time);
    let target_dt = today_naive
        .and_local_timezone(Local)
        .earliest()
        .unwrap_or_else(|| now);

    let delta: TimeDelta = if target_dt > now {
        target_dt - now
    } else {
        (target_dt + TimeDelta::days(1)) - now
    };

    Ok(Duration::from_secs(delta.num_seconds().max(1) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_seconds_and_human_durations() {
        assert_eq!(parse_duration("90").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("1h 30m").unwrap(), Duration::from_secs(5400));
    }

    #[test]
    fn rejects_bad_durations() {
        assert!(parse_duration("abc").is_err());
    }

    #[test]
    fn parses_clock_times() {
        let d = duration_until_clock("23:59").unwrap();
        assert!(d.as_secs() >= 1 && d.as_secs() <= 86400);
        assert!(duration_until_clock("99:99").is_err());
    }
}
