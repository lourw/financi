//! Calendar helpers behind the dashboard date range control (task 3.2).
//! Pure date arithmetic: nothing here reads the clock or touches GPUI;
//! the current date always arrives as the `today` parameter.

use chrono::{Datelike, Days, Months, NaiveDate};

/// Inclusive lower bound of [`DatePreset::AllTime`]: the smallest year the
/// `YYYY-MM-DD` date strings support (same sentinel as the open
/// `financi_db::ViewFilter`).
pub const ALL_TIME_FROM: NaiveDate = match NaiveDate::from_ymd_opt(1, 1, 1) {
    Some(d) => d,
    None => unreachable!(),
};

/// Inclusive upper bound of [`DatePreset::AllTime`]; pairs with
/// [`ALL_TIME_FROM`].
pub const ALL_TIME_TO: NaiveDate = match NaiveDate::from_ymd_opt(9999, 12, 31) {
    Some(d) => d,
    None => unreachable!(),
};

/// The date range presets of the dashboard filter (task 3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatePreset {
    ThisMonth,
    LastMonth,
    Last3Months,
    Last6Months,
    Last12Months,
    YearToDate,
    AllTime,
    Custom,
}

impl DatePreset {
    /// Every preset in picker order; the picker index selects one.
    pub const ALL: [DatePreset; 8] = [
        Self::ThisMonth,
        Self::LastMonth,
        Self::Last3Months,
        Self::Last6Months,
        Self::Last12Months,
        Self::YearToDate,
        Self::AllTime,
        Self::Custom,
    ];

    /// The picker label (sentence case, ellipsis for `Custom…`).
    pub fn label(self) -> &'static str {
        match self {
            Self::ThisMonth => "This month",
            Self::LastMonth => "Last month",
            Self::Last3Months => "Last 3 months",
            Self::Last6Months => "Last 6 months",
            Self::Last12Months => "Last 12 months",
            Self::YearToDate => "Year to date",
            Self::AllTime => "All time",
            Self::Custom => "Custom…",
        }
    }
}

/// The inclusive `(from, to)` calendar dates of `preset` seen from `today`.
///
/// "Last N months" are whole calendar months: the current month plus the
/// N−1 before it (on 2026-11-07, Last 3 months = 2026-09-01 … 2026-11-30).
/// "Year to date" runs from January 1 through *today*. "All time" is
/// [`ALL_TIME_FROM`] … [`ALL_TIME_TO`]. `Custom` has no range of its own:
/// callers substitute the dates the user picked, so this returns the
/// degenerate `today … today`.
pub fn preset_range(preset: DatePreset, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let this_first = today.with_day(1).unwrap();
    let this_last = last_day_of_month(today);
    match preset {
        DatePreset::ThisMonth => (this_first, this_last),
        DatePreset::LastMonth => (months_back(this_first, 1), this_first.pred_opt().unwrap()),
        DatePreset::Last3Months => (months_back(this_first, 2), this_last),
        DatePreset::Last6Months => (months_back(this_first, 5), this_last),
        DatePreset::Last12Months => (months_back(this_first, 11), this_last),
        DatePreset::YearToDate => (NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap(), today),
        DatePreset::AllTime => (ALL_TIME_FROM, ALL_TIME_TO),
        DatePreset::Custom => (today, today),
    }
}

fn months_back(first_of_month: NaiveDate, n: u32) -> NaiveDate {
    first_of_month
        .checked_sub_months(Months::new(n))
        .unwrap_or(ALL_TIME_FROM)
}

fn last_day_of_month(day: NaiveDate) -> NaiveDate {
    day.with_day(1)
        .unwrap()
        .checked_add_months(Months::new(1))
        .and_then(|next_first| next_first.checked_sub_days(Days::new(1)))
        .unwrap_or(ALL_TIME_TO)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn every_preset_from_mid_november() {
        let today = d(2026, 11, 7);
        assert_eq!(
            preset_range(DatePreset::ThisMonth, today),
            (d(2026, 11, 1), d(2026, 11, 30))
        );
        assert_eq!(
            preset_range(DatePreset::LastMonth, today),
            (d(2026, 10, 1), d(2026, 10, 31))
        );
        assert_eq!(
            preset_range(DatePreset::Last3Months, today),
            (d(2026, 9, 1), d(2026, 11, 30))
        );
        assert_eq!(
            preset_range(DatePreset::Last6Months, today),
            (d(2026, 6, 1), d(2026, 11, 30))
        );
        assert_eq!(
            preset_range(DatePreset::Last12Months, today),
            (d(2025, 12, 1), d(2026, 11, 30))
        );
        assert_eq!(
            preset_range(DatePreset::YearToDate, today),
            (d(2026, 1, 1), today)
        );
        assert_eq!(
            preset_range(DatePreset::AllTime, today),
            (ALL_TIME_FROM, ALL_TIME_TO)
        );
        assert_eq!(preset_range(DatePreset::Custom, today), (today, today));
    }

    #[test]
    fn month_end_boundaries() {
        // 31st in a month whose neighbours are 30/28/29 days long.
        assert_eq!(
            preset_range(DatePreset::ThisMonth, d(2026, 3, 31)),
            (d(2026, 3, 1), d(2026, 3, 31))
        );
        assert_eq!(
            preset_range(DatePreset::LastMonth, d(2026, 3, 31)),
            (d(2026, 2, 1), d(2026, 2, 28))
        );
        assert_eq!(
            preset_range(DatePreset::Last3Months, d(2026, 3, 31)),
            (d(2026, 1, 1), d(2026, 3, 31))
        );
        assert_eq!(
            preset_range(DatePreset::ThisMonth, d(2026, 4, 30)),
            (d(2026, 4, 1), d(2026, 4, 30))
        );
    }

    #[test]
    fn december_to_january() {
        let today = d(2027, 1, 5);
        assert_eq!(
            preset_range(DatePreset::LastMonth, today),
            (d(2026, 12, 1), d(2026, 12, 31))
        );
        assert_eq!(
            preset_range(DatePreset::Last12Months, today),
            (d(2026, 2, 1), d(2027, 1, 31))
        );
        assert_eq!(
            preset_range(DatePreset::YearToDate, today),
            (d(2027, 1, 1), today)
        );
        assert_eq!(
            preset_range(DatePreset::Last12Months, d(2026, 12, 15)),
            (d(2026, 1, 1), d(2026, 12, 31))
        );
    }

    #[test]
    fn leap_february() {
        assert_eq!(
            preset_range(DatePreset::ThisMonth, d(2028, 2, 10)),
            (d(2028, 2, 1), d(2028, 2, 29))
        );
        assert_eq!(
            preset_range(DatePreset::LastMonth, d(2028, 3, 15)),
            (d(2028, 2, 1), d(2028, 2, 29))
        );
    }

    #[test]
    fn start_of_month_and_year() {
        let first = d(2026, 11, 1);
        assert_eq!(
            preset_range(DatePreset::ThisMonth, first),
            (first, d(2026, 11, 30))
        );
        assert_eq!(
            preset_range(DatePreset::LastMonth, first),
            (d(2026, 10, 1), d(2026, 10, 31))
        );
        let new_year = d(2027, 1, 1);
        assert_eq!(
            preset_range(DatePreset::YearToDate, new_year),
            (new_year, new_year)
        );
    }
}
