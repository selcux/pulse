//! The inclusive date range that syncs and queries take.

use chrono::NaiveDate;

/// An inclusive range of calendar dates: what a sync fetches and what a query returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    /// First date (inclusive).
    pub start: NaiveDate,
    /// Last date (inclusive).
    pub end: NaiveDate,
}

impl DateRange {
    /// `end` and the `days` days before it, `days + 1` dates in all. This is what `--days` selects:
    /// `days_ending(d, 0)` is `d` alone. The start saturates at `NaiveDate::MIN` instead of panicking.
    pub fn days_ending(end: NaiveDate, days: u32) -> DateRange {
        let start = end
            .checked_sub_days(chrono::Days::new(u64::from(days)))
            .unwrap_or(NaiveDate::MIN);
        DateRange { start, end }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn days_ending_covers_the_end_and_the_days_before_it() {
        assert_eq!(
            DateRange::days_ending(date("2026-10-17"), 7),
            DateRange {
                start: date("2026-10-10"),
                end: date("2026-10-17"),
            }
        );
    }

    #[test]
    fn days_ending_zero_is_the_end_day_alone() {
        let range = DateRange::days_ending(date("2026-10-17"), 0);
        assert_eq!(range.start, date("2026-10-17"));
        assert_eq!(range.end, date("2026-10-17"));
    }

    #[test]
    fn days_ending_crosses_month_and_year_boundaries() {
        assert_eq!(
            DateRange::days_ending(date("2026-03-01"), 1).start,
            date("2026-02-28")
        );
        assert_eq!(
            DateRange::days_ending(date("2026-01-01"), 1).start,
            date("2025-12-31")
        );
    }

    #[test]
    fn days_ending_saturates_at_the_earliest_date() {
        let range = DateRange::days_ending(date("2026-10-17"), u32::MAX);
        assert_eq!(range.start, NaiveDate::MIN);
        assert_eq!(range.end, date("2026-10-17"));
    }
}
