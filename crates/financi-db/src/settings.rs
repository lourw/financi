//! Typed helpers for the `settings (key, value)` table (task 3.2): a
//! generic get/set plus the persisted dashboard date range.
//!
//! Value format for [`UI_DATE_RANGE`] (readable by design):
//!
//! - a preset code: `this`, `last1`, `last3`, `last6`, `last12`, `ytd`,
//!   `all`;
//! - or a custom range `custom:<from>..<to>` with both dates ISO
//!   (`YYYY-MM-DD`).
//!
//! A missing, unknown or malformed value falls back to the default
//! (Last 3 months) instead of erroring.

use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension};

use financi_core::dates::{DatePreset, preset_range};

use crate::Result;

/// The `settings` key holding the dashboard date range control state.
pub const UI_DATE_RANGE: &str = "ui.date_range";

/// Read a raw setting value.
pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    let value = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?;
    Ok(value)
}

/// Insert or replace a raw setting value.
pub fn set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        [key, value],
    )?;
    Ok(())
}

/// The dashboard date range control state: a preset, or two explicit
/// dates when the user picked "Custom…". [`DatePreset::Custom`] is not a
/// valid [`Self::Preset`] payload (use [`Self::Custom`] instead); it
/// encodes and decodes as the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateRange {
    Preset(DatePreset),
    Custom { from: NaiveDate, to: NaiveDate },
}

impl Default for DateRange {
    fn default() -> Self {
        Self::Preset(DatePreset::Last3Months)
    }
}

impl DateRange {
    /// The preset shown by the picker (`Custom…` for a custom range).
    pub fn preset(&self) -> DatePreset {
        match self {
            Self::Preset(p) => *p,
            Self::Custom { .. } => DatePreset::Custom,
        }
    }

    /// The inclusive `(from, to)` dates as of `today`.
    pub fn range(&self, today: NaiveDate) -> (NaiveDate, NaiveDate) {
        match self {
            Self::Preset(p) => preset_range(*p, today),
            Self::Custom { from, to } => (*from, *to),
        }
    }

    /// The [`UI_DATE_RANGE`] value for this state. `Preset(Custom)` is not
    /// a storable state and encodes as the default.
    pub fn encode(&self) -> String {
        match self {
            Self::Custom { from, to } => format!("custom:{from}..{to}"),
            Self::Preset(preset) => {
                let code = match preset {
                    DatePreset::ThisMonth => "this",
                    DatePreset::LastMonth => "last1",
                    DatePreset::Last3Months => "last3",
                    DatePreset::Last6Months => "last6",
                    DatePreset::Last12Months => "last12",
                    DatePreset::YearToDate => "ytd",
                    DatePreset::AllTime => "all",
                    DatePreset::Custom => return Self::default().encode(),
                };
                code.to_string()
            }
        }
    }

    /// Parse a [`UI_DATE_RANGE`] value; unknown or malformed input yields
    /// the default (Last 3 months).
    pub fn decode(value: &str) -> Self {
        if let Some(rest) = value.strip_prefix("custom:") {
            let (from, to) = match rest.split_once("..") {
                Some((from, to)) => (parse_date(from), parse_date(to)),
                None => (None, None),
            };
            return match (from, to) {
                (Some(from), Some(to)) => Self::Custom { from, to },
                _ => Self::default(),
            };
        }
        let preset = match value {
            "this" => DatePreset::ThisMonth,
            "last1" => DatePreset::LastMonth,
            "last3" => DatePreset::Last3Months,
            "last6" => DatePreset::Last6Months,
            "last12" => DatePreset::Last12Months,
            "ytd" => DatePreset::YearToDate,
            "all" => DatePreset::AllTime,
            _ => return Self::default(),
        };
        Self::Preset(preset)
    }
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// The persisted dashboard date range; the default when unset or the
/// stored value is not understood.
pub fn date_range(conn: &Connection) -> Result<DateRange> {
    Ok(get(conn, UI_DATE_RANGE)
        .map(|value| match value {
            Some(value) => DateRange::decode(&value),
            None => DateRange::default(),
        })
        .unwrap_or_default())
}

/// Store the dashboard date range under [`UI_DATE_RANGE`].
pub fn set_date_range(conn: &Connection, range: &DateRange) -> Result<()> {
    set(conn, UI_DATE_RANGE, &range.encode())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_in_memory;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn round_trips_every_variant() {
        let cases = [
            (DateRange::Preset(DatePreset::ThisMonth), "this"),
            (DateRange::Preset(DatePreset::LastMonth), "last1"),
            (DateRange::Preset(DatePreset::Last3Months), "last3"),
            (DateRange::Preset(DatePreset::Last6Months), "last6"),
            (DateRange::Preset(DatePreset::Last12Months), "last12"),
            (DateRange::Preset(DatePreset::YearToDate), "ytd"),
            (DateRange::Preset(DatePreset::AllTime), "all"),
            (
                DateRange::Custom {
                    from: d(2026, 9, 1),
                    to: d(2026, 11, 30),
                },
                "custom:2026-09-01..2026-11-30",
            ),
        ];
        for (range, value) in cases {
            assert_eq!(range.encode(), value);
            assert_eq!(DateRange::decode(value), range);
        }
    }

    #[test]
    fn garbage_falls_back_to_default() {
        for value in [
            "",
            "last13",
            "LAST3",
            "custom:2026-09-01",
            "custom:2026-09-01..not-a-date",
            "custom:2026-13-01..2026-11-30",
            "custom:2026-09-01:2026-11-30",
            "everything",
        ] {
            assert_eq!(DateRange::decode(value), DateRange::default(), "{value:?}");
        }
        // The invalid Preset(Custom) state encodes as the default code.
        assert_eq!(
            DateRange::Preset(DatePreset::Custom).encode(),
            DateRange::default().encode()
        );
    }

    #[test]
    fn stored_range_survives_a_restart_round_trip() {
        let conn = open_in_memory().unwrap();
        // Nothing stored yet: the default, Last 3 months.
        assert_eq!(date_range(&conn).unwrap(), DateRange::default());

        let custom = DateRange::Custom {
            from: d(2026, 3, 1),
            to: d(2026, 3, 31),
        };
        set_date_range(&conn, &custom).unwrap();
        assert_eq!(date_range(&conn).unwrap(), custom);
        assert_eq!(
            get(&conn, UI_DATE_RANGE).unwrap().as_deref(),
            Some("custom:2026-03-01..2026-03-31")
        );

        // Overwriting replaces rather than appends, and raw get/set work.
        set_date_range(&conn, &DateRange::Preset(DatePreset::AllTime)).unwrap();
        assert_eq!(
            date_range(&conn).unwrap(),
            DateRange::Preset(DatePreset::AllTime)
        );
        set(&conn, "ui.x", "1").unwrap();
        set(&conn, "ui.x", "2").unwrap();
        assert_eq!(get(&conn, "ui.x").unwrap().as_deref(), Some("2"));
    }
}
