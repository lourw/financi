//! Small parsing helpers shared by institution parsers. Parsing happens
//! after hashing, so these may trim and normalize freely.

use crate::Cents;

/// Parse a money string into cents: `"15.74"`, `"-2042.40"`, `"1,234.5"`,
/// `"$12"`, `" 101.19 "`. At most two decimal places. Empty -> `None`.
/// Commas are accepted only as thousands groups: `"1,234.56"` parses,
/// `"1,2,3"` and `"12,34"` do not (3.67).
pub fn cents(s: &str) -> Option<Cents> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (negative, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let s = s.strip_prefix('$').unwrap_or(s);
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if (whole.is_empty() && frac.is_empty())
        || !thousands_grouped(whole)
        || frac.len() > 2
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole: Cents = if whole.is_empty() {
        0
    } else {
        whole.replace(',', "").parse().ok()?
    };
    let frac: Cents = format!("{frac:0<2}").parse().ok()?;
    let value = whole.checked_mul(100)?.checked_add(frac)?;
    Some(if negative { -value } else { value })
}

/// True for digits with optional thousands separators: `1`, `1234`,
/// `12,345`, `1,234,567`. Empty is allowed (a bare `.99` has no whole
/// part). With commas, the first group holds 1..=3 digits and every
/// later group exactly 3.
fn thousands_grouped(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if !s.contains(',') {
        return s.bytes().all(|b| b.is_ascii_digit());
    }
    let mut groups = s.split(',');
    let first = groups.next().unwrap_or_default();
    if first.is_empty() || first.len() > 3 || !first.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    groups.all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()))
}

/// A calendar date as ISO `yyyy-mm-dd`, validated.
fn iso(year: u32, month: u32, day: u32) -> Option<String> {
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if !(1..=days).contains(&day) || !(1900..=2999).contains(&year) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// `"05 Nov 2026"` -> `"2026-11-05"` (Amex). Month names are English
/// three-letter abbreviations, any case.
pub fn date_dd_mon_yyyy(s: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let mut parts = s.split_whitespace();
    let day = parts.next()?.parse().ok()?;
    let mon = parts.next()?.to_ascii_lowercase();
    let year = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let month = MONTHS.iter().position(|m| *m == mon)? as u32 + 1;
    iso(year, month, day)
}

/// `"10/07/2026"` -> `"2026-10-07"` (month first, Simplii).
pub fn date_mm_dd_yyyy(s: &str) -> Option<String> {
    let mut parts = s.trim().split('/');
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    let year = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    iso(year, month, day)
}

/// The merchant part of a padded description: text before the first run
/// of two or more spaces, trimmed. `"COFFEE    VANCOUVER"` -> `"COFFEE"`.
pub fn merchant(description: &str) -> String {
    let d = description.trim();
    d.split("  ").next().unwrap_or(d).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cents() {
        assert_eq!(cents("15.74"), Some(1574));
        assert_eq!(cents("-2042.40"), Some(-204_240));
        assert_eq!(cents("1,234.5"), Some(123_450));
        assert_eq!(cents("$12"), Some(1200));
        assert_eq!(cents(" 101.19 "), Some(10_119));
        assert_eq!(cents(".99"), Some(99));
        assert_eq!(cents("-0.50"), Some(-50));
        assert_eq!(cents("1,234,567.89"), Some(123_456_789));
        assert_eq!(cents("-$1,234.56"), Some(-123_456), "quoted amount");
        assert_eq!(cents("1234"), Some(123_400), "ungrouped is fine");
    }

    #[test]
    fn rejects_bad_money() {
        for bad in ["", "  ", "abc", "1.234", "1.2.3", "--1", "1e5", "-", "."] {
            assert_eq!(cents(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn commas_must_be_thousands_groups() {
        for bad in [
            "1,2,3",
            "12,34",
            "1234,5678",
            "1,23",
            ",123",
            "1,",
            "1,,234",
        ] {
            assert_eq!(cents(bad), None, "{bad:?}");
        }
        assert_eq!(cents("1,234"), Some(123_400));
        assert_eq!(cents("12,345,678"), Some(1_234_567_800));
    }

    #[test]
    fn parses_amex_dates() {
        assert_eq!(
            date_dd_mon_yyyy("05 Nov 2026").as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(
            date_dd_mon_yyyy("29 FEB 2028").as_deref(),
            Some("2028-02-29")
        );
        assert_eq!(date_dd_mon_yyyy("29 Feb 2026"), None, "not a leap year");
        assert_eq!(date_dd_mon_yyyy("31 Apr 2026"), None);
        assert_eq!(date_dd_mon_yyyy("05 Nov"), None);
        assert_eq!(date_dd_mon_yyyy("05 Novem 2026"), None);
    }

    #[test]
    fn parses_month_first_dates() {
        assert_eq!(date_mm_dd_yyyy("10/07/2026").as_deref(), Some("2026-10-07"));
        assert_eq!(date_mm_dd_yyyy("1/2/2026").as_deref(), Some("2026-01-02"));
        assert_eq!(date_mm_dd_yyyy("13/01/2026"), None, "month first");
        assert_eq!(date_mm_dd_yyyy("2026-10-07"), None);
    }

    #[test]
    fn extracts_merchant() {
        assert_eq!(merchant("COFFEE    VANCOUVER"), "COFFEE");
        assert_eq!(merchant("LYFT   *RIDE 11-05  VANCOUVER"), "LYFT");
        assert_eq!(merchant(" INTERAC E-TRANSFER"), "INTERAC E-TRANSFER");
    }
}
