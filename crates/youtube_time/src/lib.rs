//! Small, dependency-free parsers for the two timestamp formats used by YouTube.

/// Parses an RFC 3339 date-time into whole Unix seconds.
///
/// Fractional seconds are accepted and truncated because the search schema stores seconds.
pub fn parse_rfc3339_timestamp(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() < 20 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    if !matches!(bytes.get(10), Some(b'T' | b't' | b' '))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }

    let year = decimal(bytes, 0, 4)? as i64;
    let month = decimal(bytes, 5, 2)?;
    let day = decimal(bytes, 8, 2)?;
    let hour = decimal(bytes, 11, 2)?;
    let minute = decimal(bytes, 14, 2)?;
    let second = decimal(bytes, 17, 2)?;
    if year == 0
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    let mut cursor = 19;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let fraction_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == fraction_start {
            return None;
        }
    }

    let offset_seconds = match bytes.get(cursor) {
        Some(b'Z' | b'z') if cursor + 1 == bytes.len() => 0_i64,
        Some(sign @ (b'+' | b'-')) if cursor + 6 == bytes.len() => {
            if bytes.get(cursor + 3) != Some(&b':') {
                return None;
            }
            let offset_hour = decimal(bytes, cursor + 1, 2)?;
            let offset_minute = decimal(bytes, cursor + 4, 2)?;
            if offset_hour > 23 || offset_minute > 59 {
                return None;
            }
            let offset = i64::from(offset_hour * 3600 + offset_minute * 60);
            if *sign == b'+' { offset } else { -offset }
        }
        _ => return None,
    };

    let days = days_from_civil(year, month, day);
    days.checked_mul(86_400)?
        .checked_add(i64::from(hour * 3600 + minute * 60 + second))?
        .checked_sub(offset_seconds)
}

/// Parses YouTube's ISO-8601 `PT#H#M#S` video duration into milliseconds.
pub fn parse_duration_ms(value: &str) -> Option<u32> {
    let value = value.strip_prefix("PT")?;
    if value.is_empty() {
        return None;
    }
    let mut amount = 0_u32;
    let mut has_amount = false;
    let mut millis = 0_u32;
    let mut last_unit = 0_u8;

    for byte in value.bytes() {
        if byte.is_ascii_digit() {
            has_amount = true;
            amount = amount
                .checked_mul(10)?
                .checked_add(u32::from(byte - b'0'))?;
            continue;
        }
        if !has_amount {
            return None;
        }
        let (unit_order, unit_millis) = match byte {
            b'H' => (1, 3_600_000_u32),
            b'M' => (2, 60_000_u32),
            b'S' => (3, 1_000_u32),
            _ => return None,
        };
        if unit_order <= last_unit {
            return None;
        }
        millis = millis.checked_add(amount.checked_mul(unit_millis)?)?;
        amount = 0;
        has_amount = false;
        last_unit = unit_order;
    }

    (!has_amount).then_some(millis)
}

fn decimal(bytes: &[u8], start: usize, len: usize) -> Option<u32> {
    let mut value = 0_u32;
    for byte in bytes.get(start..start.checked_add(len)?)? {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .checked_mul(10)?
            .checked_add(u32::from(*byte - b'0'))?;
    }
    Some(value)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

// Howard Hinnant's civil-calendar conversion, shifted to the Unix epoch.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rfc3339_utc_offsets_and_fractional_seconds() {
        assert_eq!(parse_rfc3339_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_timestamp("1969-12-31T23:59:59Z"), Some(-1));
        assert_eq!(
            parse_rfc3339_timestamp("2024-02-29T12:34:56.789Z"),
            Some(1_709_210_096)
        );
        assert_eq!(
            parse_rfc3339_timestamp("1970-01-01T01:00:00+01:00"),
            Some(0)
        );
        assert_eq!(
            parse_rfc3339_timestamp("1969-12-31T19:00:00-05:00"),
            Some(0)
        );
    }

    #[test]
    fn rejects_invalid_rfc3339_values() {
        for value in [
            "",
            "2024-02-30T00:00:00Z",
            "2023-02-29T00:00:00Z",
            "2024-01-01T24:00:00Z",
            "2024-01-01T00:60:00Z",
            "2024-01-01T00:00:60Z",
            "2024-01-01T00:00:00",
            "2024-01-01T00:00:00+24:00",
        ] {
            assert_eq!(parse_rfc3339_timestamp(value), None, "accepted {value}");
        }
    }

    #[test]
    fn parses_youtube_durations() {
        assert_eq!(parse_duration_ms("PT1H2M3S"), Some(3_723_000));
        assert_eq!(parse_duration_ms("PT4M5S"), Some(245_000));
        assert_eq!(parse_duration_ms("PT30S"), Some(30_000));
        assert_eq!(parse_duration_ms("PT0S"), Some(0));
    }

    #[test]
    fn rejects_invalid_or_overflowing_durations() {
        for value in ["", "PT", "P1D", "PT1S2M", "PT1M2M", "PT1.5S", "PT4294968S"] {
            assert_eq!(parse_duration_ms(value), None, "accepted {value}");
        }
    }
}
