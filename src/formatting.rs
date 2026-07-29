//! Shared formatting, parsing, and comparison utilities.
//!
//! This module consolidates functions that were duplicated across CLI pickers,
//! TUI rendering, and data modules.

use chrono::{DateTime, Utc};
use std::cmp::Ordering;
use std::fmt::Display;

/// Em-dash sentinel for missing values (style guide mandated).
pub(crate) const EM_DASH: &str = "\u{2014}";

/// Display an `Option` value, falling back to an em-dash for `None`.
#[allow(dead_code)]
pub(crate) fn or_em_dash(opt: Option<impl Display>) -> String {
    match opt {
        Some(v) => v.to_string(),
        None => EM_DASH.to_string(),
    }
}

/// Truncate `s` to at most `max_chars` characters, appending "..." if truncated.
/// Uses `chars().count()` (not `.len()`) for UTF-8 safety.
pub(crate) fn truncate(s: &str, max_chars: usize) -> String {
    if max_chars <= 3 {
        return s.chars().take(max_chars).collect();
    }
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 3).collect();
        format!("{}...", truncated)
    }
}

/// Format a token count: `128000` → `"128k"`, `1500000` → `"1.5M"`, `2000000` → `"2M"`.
/// Whole values omit the decimal; sub-1k values render as raw numbers.
pub(crate) fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        if m.fract() == 0.0 {
            format!("{}M", m as u64)
        } else {
            format!("{:.1}M", m)
        }
    } else if n >= 1_000 {
        let k = n as f64 / 1_000.0;
        if k.fract() == 0.0 {
            format!("{}k", k as u64)
        } else {
            format!("{:.1}k", k)
        }
    } else {
        n.to_string()
    }
}

/// Format a star/download count: `1234` → `"1.2k"`, `1234567` → `"1.2m"`, `42` → `"42"`.
pub(crate) fn format_stars(stars: u64) -> String {
    if stars >= 1_000_000 {
        format!("{:.1}m", stars as f64 / 1_000_000.0)
    } else if stars >= 1_000 {
        format!("{:.1}k", stars as f64 / 1_000.0)
    } else {
        stars.to_string()
    }
}

/// Parse a date/datetime string. Accepts RFC3339 (with any offset), ISO 8601 date ("YYYY-MM-DD"),
/// and other chrono-recognized formats. Returns UTC.
pub(crate) fn parse_date(date_str: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = date_str.parse::<DateTime<Utc>>() {
        return Some(dt);
    }
    chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| DateTime::from_naive_utc_and_offset(ndt, Utc))
}

/// Format a `DateTime<Utc>` as relative time: `"2h ago"`, `"3d ago"`, `"1mo ago"`.
pub(crate) fn format_relative_time(dt: &DateTime<Utc>) -> String {
    let delta = Utc::now().signed_duration_since(*dt);
    let minutes = delta.num_minutes();
    let hours = delta.num_hours();
    let days = delta.num_days();
    let weeks = days / 7;
    let months = days / 30;

    if months > 0 {
        format!("{}mo ago", months)
    } else if weeks > 0 {
        format!("{}w ago", weeks)
    } else if days > 0 {
        format!("{}d ago", days)
    } else if hours > 0 {
        format!("{}h ago", hours)
    } else {
        format!("{}m ago", minutes.max(1))
    }
}

/// Convenience: parse a timestamp string and format as relative time.
/// Falls back to the raw string if parsing fails.
pub(crate) fn format_relative_time_from_str(ts: &str) -> String {
    parse_date(ts)
        .map(|dt| format_relative_time(&dt))
        .unwrap_or_else(|| ts.to_string())
}

/// Parse "YYYY-MM-DD" to a sortable numeric value (e.g., `20240115.0`).
/// Used for sorting by date in table columns.
pub(crate) fn parse_date_to_numeric(date_str: &str) -> Option<f64> {
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() == 3 {
        let year: f64 = parts[0].parse().ok()?;
        let month: f64 = parts[1].parse().ok()?;
        let day: f64 = parts[2].parse().ok()?;
        Some(year * 10000.0 + month * 100.0 + day)
    } else {
        None
    }
}

/// Format a dollar amount with tiered precision and no trailing fractional
/// zeros: `150.0` → `"$150"`, `5.0` → `"$5"`, `2.50` → `"$2.5"`,
/// `1.25` → `"$1.25"`, `0.075` → `"$0.075"`. A decimal digit only appears
/// when it carries information — `$5.00` spends space on noise.
pub(crate) fn format_usd(v: f64) -> String {
    let s = if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 1.0 {
        format!("{v:.2}")
    } else {
        // Sub-dollar prices carry signal in the third decimal ($0.075
        // cache reads); the trim keeps $0.3 from becoming $0.300.
        format!("{v:.3}")
    };
    let trimmed = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s.as_str()
    };
    format!("${trimmed}")
}

/// Compare two `Option<f64>` values. `None` sorts last (Greater).
pub(crate) fn cmp_opt_f64(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_or_em_dash_some() {
        assert_eq!(or_em_dash(Some(42)), "42");
        assert_eq!(or_em_dash(Some("hello")), "hello");
    }

    #[test]
    fn test_or_em_dash_none() {
        assert_eq!(or_em_dash(None::<String>), "\u{2014}");
    }

    #[test]
    fn test_format_usd_trims_noise_zeros() {
        assert_eq!(format_usd(150.0), "$150");
        assert_eq!(format_usd(5.0), "$5");
        assert_eq!(format_usd(2.5), "$2.5");
        assert_eq!(format_usd(1.25), "$1.25");
        assert_eq!(format_usd(0.3), "$0.3");
        assert_eq!(format_usd(0.075), "$0.075");
        assert_eq!(format_usd(0.0), "$0");
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn test_truncate_exact() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_long() {
        assert_eq!(truncate("hello world", 8), "hello...");
    }

    #[test]
    fn test_truncate_tiny_max() {
        assert_eq!(truncate("hello", 3), "hel");
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(1000), "1k");
        assert_eq!(format_tokens(128000), "128k");
        assert_eq!(format_tokens(1500000), "1.5M");
        assert_eq!(format_tokens(2000000), "2M");
    }

    #[test]
    fn test_format_stars() {
        assert_eq!(format_stars(0), "0");
        assert_eq!(format_stars(999), "999");
        assert_eq!(format_stars(1000), "1.0k");
        assert_eq!(format_stars(1234567), "1.2m");
    }

    #[test]
    fn format_relative_time_minutes() {
        let dt = Utc::now() - Duration::minutes(5);
        assert_eq!(format_relative_time(&dt), "5m ago");
    }

    #[test]
    fn format_relative_time_minimum_one_minute() {
        let dt = Utc::now() - Duration::seconds(10);
        assert_eq!(format_relative_time(&dt), "1m ago");
    }

    #[test]
    fn format_relative_time_hours() {
        let dt = Utc::now() - Duration::hours(3);
        assert_eq!(format_relative_time(&dt), "3h ago");
    }

    #[test]
    fn format_relative_time_days() {
        let dt = Utc::now() - Duration::days(2);
        assert_eq!(format_relative_time(&dt), "2d ago");
    }

    #[test]
    fn format_relative_time_weeks() {
        let dt = Utc::now() - Duration::weeks(3);
        assert_eq!(format_relative_time(&dt), "3w ago");
    }

    #[test]
    fn format_relative_time_months() {
        let dt = Utc::now() - Duration::days(90);
        assert_eq!(format_relative_time(&dt), "3mo ago");
    }

    #[test]
    fn parse_date_iso() {
        let result = parse_date("2024-06-15");
        assert!(result.is_some());
        let dt = result.unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2024-06-15");
    }

    #[test]
    fn parse_date_rfc3339() {
        let result = parse_date("2024-06-15T12:30:00Z");
        assert!(result.is_some());
        let dt = result.unwrap();
        assert_eq!(
            dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "2024-06-15T12:30:00"
        );
    }

    #[test]
    fn parse_date_rfc3339_positive_offset() {
        let result = parse_date("2024-06-15T12:30:00+05:30");
        assert!(result.is_some());
        let dt = result.unwrap();
        assert_eq!(
            dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "2024-06-15T07:00:00"
        );
    }

    #[test]
    fn parse_date_rfc3339_negative_offset() {
        let result = parse_date("2024-06-15T12:30:00-07:00");
        assert!(result.is_some());
        let dt = result.unwrap();
        assert_eq!(
            dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "2024-06-15T19:30:00"
        );
    }

    #[test]
    fn parse_date_invalid() {
        assert!(parse_date("not-a-date").is_none());
        assert!(parse_date("").is_none());
    }

    #[test]
    fn test_parse_date_to_numeric() {
        assert_eq!(parse_date_to_numeric("2024-01-15"), Some(20240115.0));
        assert_eq!(parse_date_to_numeric("not-a-date"), None);
    }

    #[test]
    fn test_cmp_opt_f64() {
        assert_eq!(cmp_opt_f64(Some(1.0), Some(2.0)), Ordering::Less);
        assert_eq!(cmp_opt_f64(Some(2.0), Some(1.0)), Ordering::Greater);
        assert_eq!(cmp_opt_f64(Some(1.0), None), Ordering::Less);
        assert_eq!(cmp_opt_f64(None, Some(1.0)), Ordering::Greater);
        assert_eq!(cmp_opt_f64(None, None), Ordering::Equal);
    }
}
