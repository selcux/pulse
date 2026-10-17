//! Date range flags: --days, --from and --to on the read commands and on sync.

use std::num::NonZeroU32;

use chrono::NaiveDate;
use pulse_health_core::date_range::DateRange;

/// Default `--days` of the read commands: one week.
const READ_DEFAULT_DAYS: NonZeroU32 = NonZeroU32::new(7).unwrap();
/// Default `--days` of `sync`: today and yesterday, because Garmin finalises last night's sleep
/// after waking.
const SYNC_DEFAULT_DAYS: NonZeroU32 = NonZeroU32::new(2).unwrap();

/// `--days`, `--from` and `--to` of the read commands (`sleep` ... `workouts`, `vitality`).
#[derive(Debug, Clone, Copy, clap::Args)]
pub struct RangeArgs {
    /// Show the last N days, today included
    #[arg(
        short, long, value_name = "N", default_value_t = READ_DEFAULT_DAYS,
        value_parser = parse_days, conflicts_with_all = ["from", "to"]
    )]
    days: NonZeroU32,
    /// First day to show, YYYY-MM-DD
    #[arg(long, value_name = "DATE", value_parser = parse_iso_date)]
    from: Option<NaiveDate>,
    /// Last day to show, YYYY-MM-DD (default: today). Needs --from
    #[arg(long, value_name = "DATE", value_parser = parse_iso_date, requires = "from")]
    to: Option<NaiveDate>,
}

/// The same flags on `sync`: `--days` defaults to 2, and the range cannot end after today.
#[derive(Debug, Clone, Copy, clap::Args)]
pub struct SyncRangeArgs {
    /// Sync the last N days, today included
    #[arg(
        short, long, value_name = "N", default_value_t = SYNC_DEFAULT_DAYS,
        value_parser = parse_days, conflicts_with_all = ["from", "to"]
    )]
    days: NonZeroU32,
    /// First day to sync, YYYY-MM-DD
    #[arg(long, value_name = "DATE", value_parser = parse_iso_date)]
    from: Option<NaiveDate>,
    /// Last day to sync, YYYY-MM-DD, not after today (default: today). Needs --from
    #[arg(long, value_name = "DATE", value_parser = parse_iso_date, requires = "from")]
    to: Option<NaiveDate>,
}

impl RangeArgs {
    /// The dates to show. Error messages name the flags; `parse_cli` reports them as clap errors.
    pub fn resolve(self, today: NaiveDate) -> anyhow::Result<DateRange> {
        resolve(self.days, self.from, self.to, today, true)
    }
}

impl SyncRangeArgs {
    /// The dates to sync. Like [`RangeArgs::resolve`], and the range cannot end after `today`.
    pub fn resolve(self, today: NaiveDate) -> anyhow::Result<DateRange> {
        resolve(self.days, self.from, self.to, today, false)
    }
}

/// Shared by both `resolve`s. Without `from` the range is the last `days` dates ending `today` (clap
/// guarantees `to` is `None` then); with it, `from ..= to`, and `to` defaults to `today`.
fn resolve(
    days: NonZeroU32,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    today: NaiveDate,
    allow_future: bool,
) -> anyhow::Result<DateRange> {
    let Some(start) = from else {
        return Ok(DateRange::last_days(today, days));
    };
    if !allow_future
        && let Some(to) = to
        && to > today
    {
        anyhow::bail!("--to {to} is after today ({today}); sync cannot fetch future days");
    }
    let end = to.unwrap_or(today);
    if start > end {
        match to {
            Some(to) => anyhow::bail!("--from {start} is after --to {to}"),
            None => anyhow::bail!("--from {start} is after today ({today})"),
        }
    }
    Ok(DateRange { start, end })
}

/// Clap value parser for `--days`: a whole number of at least 1.
fn parse_days(s: &str) -> Result<NonZeroU32, String> {
    let days: u32 = s
        .parse()
        .map_err(|_| format!("'{s}' is not a whole number of days"))?;
    NonZeroU32::new(days).ok_or_else(|| "must be at least 1".to_string())
}

/// Clap value parser for `--from` and `--to`: exactly `YYYY-MM-DD`.
///
/// Chrono alone also accepts `2026-1-5` and signed years such as `+10000-01-01`, whose text sorts
/// before every digit and would make the `BETWEEN` queries on the stored date text match nothing.
fn parse_iso_date(s: &str) -> Result<NaiveDate, String> {
    const FORMAT: &str = "%Y-%m-%d";
    let date =
        NaiveDate::parse_from_str(s, FORMAT).map_err(|e| format!("{e}; expected YYYY-MM-DD"))?;
    if s.len() != 10 || date.format(FORMAT).to_string() != s {
        return Err("expected YYYY-MM-DD".to_string());
    }
    Ok(date)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        date("2026-10-17")
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn range(start: &str, end: &str) -> DateRange {
        DateRange {
            start: date(start),
            end: date(end),
        }
    }

    fn read(days: u32, from: Option<&str>, to: Option<&str>) -> RangeArgs {
        RangeArgs {
            days: NonZeroU32::new(days).unwrap(),
            from: from.map(date),
            to: to.map(date),
        }
    }

    fn sync(days: u32, from: Option<&str>, to: Option<&str>) -> SyncRangeArgs {
        SyncRangeArgs {
            days: NonZeroU32::new(days).unwrap(),
            from: from.map(date),
            to: to.map(date),
        }
    }

    #[test]
    fn days_are_that_many_dates_ending_today() {
        assert_eq!(
            read(7, None, None).resolve(today()).unwrap(),
            range("2026-10-11", "2026-10-17")
        );
        assert_eq!(
            read(1, None, None).resolve(today()).unwrap(),
            range("2026-10-17", "2026-10-17")
        );
        assert_eq!(
            sync(1, None, None).resolve(today()).unwrap(),
            range("2026-10-17", "2026-10-17")
        );
        assert_eq!(
            sync(2, None, None).resolve(today()).unwrap(),
            range("2026-10-16", "2026-10-17")
        );
    }

    #[test]
    fn days_must_be_a_whole_number_of_at_least_one() {
        assert_eq!(parse_days("1"), Ok(NonZeroU32::new(1).unwrap()));
        assert_eq!(parse_days("30"), Ok(NonZeroU32::new(30).unwrap()));
        assert_eq!(parse_days("0"), Err("must be at least 1".to_string()));
        for s in ["-1", "1.5", "", "seven"] {
            let err = parse_days(s).expect_err(s);
            assert!(
                err.ends_with("is not a whole number of days"),
                "{s:?}: {err}"
            );
        }
    }

    #[test]
    fn from_alone_ends_today() {
        assert_eq!(
            read(7, Some("2026-10-01"), None).resolve(today()).unwrap(),
            range("2026-10-01", "2026-10-17")
        );
    }

    #[test]
    fn from_and_to_give_exactly_that_range() {
        let first_week = range("2026-10-01", "2026-10-07");
        assert_eq!(
            read(7, Some("2026-10-01"), Some("2026-10-07"))
                .resolve(today())
                .unwrap(),
            first_week
        );
        assert_eq!(
            sync(1, Some("2026-10-01"), Some("2026-10-07"))
                .resolve(today())
                .unwrap(),
            first_week
        );
        let one_day = range("2026-10-05", "2026-10-05");
        assert_eq!(
            read(7, Some("2026-10-05"), Some("2026-10-05"))
                .resolve(today())
                .unwrap(),
            one_day
        );
        assert_eq!(
            sync(1, Some("2026-10-05"), Some("2026-10-05"))
                .resolve(today())
                .unwrap(),
            one_day
        );
    }

    #[test]
    fn from_after_to_is_an_error_naming_both_flags() {
        let message = "--from 2026-10-08 is after --to 2026-10-01";
        let err = read(7, Some("2026-10-08"), Some("2026-10-01"))
            .resolve(today())
            .unwrap_err();
        assert_eq!(err.to_string(), message);
        let err = sync(1, Some("2026-10-08"), Some("2026-10-01"))
            .resolve(today())
            .unwrap_err();
        assert_eq!(err.to_string(), message);
    }

    #[test]
    fn from_after_today_without_to_is_an_error() {
        let message = "--from 2026-10-18 is after today (2026-10-17)";
        let err = read(7, Some("2026-10-18"), None)
            .resolve(today())
            .unwrap_err();
        assert_eq!(err.to_string(), message);
        let err = sync(1, Some("2026-10-18"), None)
            .resolve(today())
            .unwrap_err();
        assert_eq!(err.to_string(), message);
    }

    #[test]
    fn read_ranges_may_end_after_today() {
        assert!(
            read(7, Some("2026-12-01"), Some("2026-12-31"))
                .resolve(today())
                .is_ok()
        );
        assert!(
            read(7, Some("2026-10-01"), Some("2026-10-31"))
                .resolve(today())
                .is_ok()
        );
    }

    #[test]
    fn sync_ranges_cannot_end_after_today() {
        let message = "--to 2026-10-18 is after today (2026-10-17); sync cannot fetch future days";
        let err = sync(1, Some("2026-10-01"), Some("2026-10-18"))
            .resolve(today())
            .unwrap_err();
        assert_eq!(err.to_string(), message);

        // The `--to` check runs before the `--from` one.
        let err = sync(1, Some("2026-12-01"), Some("2026-12-31"))
            .resolve(today())
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "--to 2026-12-31 is after today (2026-10-17); sync cannot fetch future days"
        );

        assert!(
            sync(1, Some("2026-10-01"), Some("2026-10-17"))
                .resolve(today())
                .is_ok()
        );
    }

    #[test]
    fn parse_iso_date_accepts_yyyy_mm_dd() {
        for s in ["2026-10-17", "2024-02-29", "0001-01-01"] {
            assert_eq!(parse_iso_date(s), Ok(date(s)), "{s}");
        }
    }

    #[test]
    fn parse_iso_date_rejects_everything_else() {
        for s in [
            "2026-13-01",
            "2026-02-30",
            "2025-02-29",
            "2026-1-5",
            " 2026-01-05",
            "2026-01-05 ",
            "20261017",
            "2026-10-17T00:00",
            "+10000-01-01",
            "-0001-01-01",
            "99-01-01",
            "yesterday",
            "",
        ] {
            let err = parse_iso_date(s).expect_err(s);
            assert!(err.ends_with("expected YYYY-MM-DD"), "{s:?}: {err}");
        }
    }
}
