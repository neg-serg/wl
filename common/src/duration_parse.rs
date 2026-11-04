use std::time::Duration;

/// Parse a flexible duration string into a [`Duration`].
///
/// Supported formats:
/// - Compound: `1h30m`, `2h45m30s`, `1d12h`
/// - Single unit: `30s`, `45m`, `2h`, `1d`
/// - Plain number: `90` (interpreted as minutes)
/// - Units: `s` (seconds), `m` (minutes), `h` (hours), `d` (days)
///
/// Returns an error for invalid input, zero duration, or duplicate/out-of-order units.
#[doc(hidden)]
pub const MAX_INTERVAL_SECS: u64 = 10 * 365 * 24 * 3600;

pub fn parse_duration(input: &str) -> Result<Duration, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty interval string".to_string());
    }

    // Reject negative values
    if input.starts_with('-') {
        return Err(format!(
            "invalid interval '{input}': negative values not allowed"
        ));
    }

    // Plain number (no unit suffix) → interpret as minutes
    if let Ok(n) = input.parse::<u64>() {
        let secs = n
            .checked_mul(60)
            .ok_or_else(|| format!("invalid interval '{input}': too large"))?;
        return checked_interval(input, secs);
    }

    let mut total_secs: u64 = 0;
    let mut last_unit_order: Option<u8> = None; // d=4, h=3, m=2, s=1
    let mut chars = input.chars().peekable();
    let mut found_segment = false;

    while chars.peek().is_some() {
        // Parse digits
        let mut num_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                num_str.push(c);
                chars.next();
            } else {
                break;
            }
        }

        if num_str.is_empty() {
            return Err(format!("invalid interval '{input}': expected digits"));
        }

        let value: u64 = num_str
            .parse()
            .map_err(|_| format!("invalid interval '{input}': number too large"))?;

        // Parse unit
        let unit = match chars.peek() {
            Some(&'s') => {
                chars.next();
                1
            }
            Some(&'m') => {
                chars.next();
                2
            }
            Some(&'h') => {
                chars.next();
                3
            }
            Some(&'d') => {
                chars.next();
                4
            }
            Some(&c) => {
                return Err(format!(
                    "invalid interval '{input}': unknown unit '{c}' (expected s, m, h, or d)"
                ));
            }
            None => {
                // Trailing digits without unit — not valid in compound expression
                return Err(format!(
                    "invalid interval '{input}': missing unit after '{num_str}'"
                ));
            }
        };

        // Check descending order and no duplicates
        if let Some(prev) = last_unit_order
            && unit >= prev
        {
            return Err(format!(
                "invalid interval '{input}': units must be in descending order (d > h > m > s)"
            ));
        }
        last_unit_order = Some(unit);

        let multiplier = match unit {
            1 => 1,
            2 => 60,
            3 => 3600,
            4 => 86400,
            _ => unreachable!(),
        };

        let segment_secs = value
            .checked_mul(multiplier)
            .ok_or_else(|| format!("invalid interval '{input}': too large"))?;
        total_secs = total_secs
            .checked_add(segment_secs)
            .ok_or_else(|| format!("invalid interval '{input}': too large"))?;
        found_segment = true;
    }

    if !found_segment {
        return Err(format!("invalid interval '{input}'"));
    }

    checked_interval(input, total_secs)
}

/// Validate a parsed second count and turn it into a [`Duration`].
///
/// The upper bound keeps `Instant::now() + interval` (and any libc time math)
/// representable: an unvalidated 1.3e19-second interval used to parse cleanly
/// and then panic the daemon when the rotation deadline was computed.
fn checked_interval(input: &str, secs: u64) -> Result<Duration, String> {
    if secs == 0 {
        return Err("interval must be greater than 0".to_string());
    }
    if secs > MAX_INTERVAL_SECS {
        return Err(format!(
            "invalid interval '{input}': too large (maximum is {} days)",
            MAX_INTERVAL_SECS / 86_400
        ));
    }
    Ok(Duration::from_secs(secs))
}

/// Format a duration in seconds to a human-readable string (e.g., `1h 30m`).
pub fn format_duration(secs: u64) -> String {
    if secs == 0 {
        return "0s".to_string();
    }

    let mut remaining = secs;
    let mut parts = Vec::new();

    let days = remaining / 86400;
    remaining %= 86400;
    if days > 0 {
        parts.push(format!("{days}d"));
    }

    let hours = remaining / 3600;
    remaining %= 3600;
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }

    let minutes = remaining / 60;
    remaining %= 60;
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }

    if remaining > 0 {
        parts.push(format!("{remaining}s"));
    }

    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_intervals_that_would_overflow_a_deadline() {
        // Wraps u64 when multiplied by 60/86400 if the math is unchecked.
        assert!(parse_duration("18446744073709551615d").is_err());
        assert!(parse_duration("150000000000000d").is_err());
        assert!(parse_duration("18446744073709551615").is_err());
        // 4000 days is over the 10 year cap.
        assert!(parse_duration("4000d").is_err());
        // The cap itself still parses.
        assert_eq!(
            parse_duration("3650d").unwrap(),
            Duration::from_secs(3650 * 86_400)
        );
    }

    #[test]
    fn test_single_units() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("45m").unwrap(), Duration::from_secs(45 * 60));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(2 * 3600));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
    }

    #[test]
    fn test_compound() {
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(
            parse_duration("2h45m30s").unwrap(),
            Duration::from_secs(2 * 3600 + 45 * 60 + 30)
        );
        assert_eq!(
            parse_duration("1d12h").unwrap(),
            Duration::from_secs(86400 + 12 * 3600)
        );
    }

    #[test]
    fn test_plain_number_as_minutes() {
        assert_eq!(parse_duration("90").unwrap(), Duration::from_secs(90 * 60));
        assert_eq!(parse_duration("1").unwrap(), Duration::from_secs(60));
    }

    #[test]
    fn test_invalid() {
        assert!(parse_duration("0").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("-5m").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("0s").is_err());
        assert!(parse_duration("0m").is_err());
    }

    #[test]
    fn test_duplicate_units() {
        assert!(parse_duration("1h2h").is_err());
        assert!(parse_duration("1m2m").is_err());
    }

    #[test]
    fn test_wrong_order() {
        assert!(parse_duration("30s1m").is_err());
        assert!(parse_duration("1m1h").is_err());
    }

    #[test]
    fn test_unknown_unit() {
        assert!(parse_duration("5x").is_err());
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(30), "30s");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(3600), "1h");
        assert_eq!(format_duration(5400), "1h 30m");
        assert_eq!(format_duration(86400), "1d");
        assert_eq!(format_duration(129600), "1d 12h");
    }
}
