//! Formats durations in seconds for audit entries.

/// The number with at most two decimals and no trailing zeros, as the first
/// server's `0.##` made it: the value is first read to 15 significant digits,
/// and what follows the second decimal rounds up from a half. `1.005` is
/// therefore `1.01` here, where formatting the value exactly would say `1`.
pub fn two_places(value: f64) -> String {
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let magnitude = value.abs();
    if !magnitude.is_finite() {
        return format!("{sign}{magnitude}");
    }
    // `d.dddddddddddddde±x`: fifteen digits and the power of the first.
    let written = format!("{magnitude:.14e}");
    let Some((digits, exponent)) = written.split_once('e') else {
        return "0".to_string();
    };
    let digits: String = digits.chars().filter(char::is_ascii_digit).collect();
    let Ok(exponent) = exponent.parse::<i32>() else {
        return "0".to_string();
    };
    let Ok(digits) = digits.parse::<u128>() else {
        return "0".to_string();
    };
    // The value is `digits * 10^(exponent - 14)`; hundredths are its 100 times.
    let shift = exponent - 12;
    let hundredths = if shift >= 0 {
        match 10u128.checked_pow(shift as u32).and_then(|scale| digits.checked_mul(scale)) {
            Some(exact) => exact,
            None => return format!("{sign}{magnitude}"),
        }
    } else if shift < -38 {
        0
    } else {
        let scale = 10u128.pow((-shift) as u32);
        (digits + scale / 2) / scale
    };
    let whole = hundredths / 100;
    let part = hundredths % 100;
    match part {
        // What rounds to nothing has no sign, as `-0` is not a number of seconds.
        0 if whole == 0 => "0".to_string(),
        0 => format!("{sign}{whole}"),
        _ if part % 10 == 0 => format!("{sign}{whole}.{}", part / 10),
        _ => format!("{sign}{whole}.{part:02}"),
    }
}

/// `1.25–5.5`, the text of a trim in the audit log.
pub fn range(start: f64, end: f64) -> String {
    format!("{}\u{2013}{}", two_places(start), two_places(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_keeps_at_most_two_decimals_and_loses_its_trailing_zeros() {
        for (value, written) in [
            (0.0, "0"),
            (2.0, "2"),
            (1.25, "1.25"),
            (1.5, "1.5"),
            (0.15, "0.15"),
            (0.001, "0"),
            (0.005, "0.01"),
            (0.004, "0"),
            (1.005, "1.01"),
            (2.675, "2.68"),
            (8.255, "8.26"),
            (0.125, "0.13"),
            (99.995, "100"),
            (12.0, "12"),
            (1234.5678, "1234.57"),
            (-0.0, "0"),
            (-0.001, "0"),
            (-0.004, "0"),
            (-0.005, "-0.01"),
            (-1.5, "-1.5"),
        ] {
            assert_eq!(two_places(value), written, "{value}");
        }
    }

    #[test]
    fn a_trim_is_two_numbers_joined_by_an_en_dash() {
        assert_eq!(range(1.25, 5.5), "1.25\u{2013}5.5");
    }
}
