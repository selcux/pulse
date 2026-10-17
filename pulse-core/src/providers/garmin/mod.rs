pub mod api;
pub mod auth;
pub mod tokens;

use std::collections::BTreeSet;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::db::Database;
use crate::db::queries;
use crate::models::{Activity, Heart, Recovery, Sleep, Stress};
use crate::providers::{DateRange, MetricType, Provider, SyncReport, record_raw_response};

use api::{
    GarminApi, GarminBodyBatteryResponse, GarminDailySummaryResponse, GarminEndpoint,
    GarminHrvResponse, GarminMaxMetResponse, GarminSleepResponse,
};

pub struct GarminProvider {
    client: reqwest::blocking::Client,
    raw_responses: bool,
}

impl GarminProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::ClientBuilder::new()
                .cookie_store(true)
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("Failed to build HTTP client"),
            raw_responses: true,
        }
    }

    /// Keep (`true`, the default) or stop keeping each response body in `raw_responses`.
    pub fn with_raw_responses(mut self, enabled: bool) -> Self {
        self.raw_responses = enabled;
        self
    }

    /// Perform login. Delegates to auth module.
    pub fn login(
        &self,
        email: &str,
        password: &str,
        mfa_code: Option<&str>,
    ) -> Result<auth::LoginResult> {
        auth::login(&self.client, email, password, mfa_code)
    }

    // -----------------------------------------------------------------------
    // Mapping helpers
    // -----------------------------------------------------------------------

    fn map_sleep(
        date: NaiveDate,
        resp: &GarminSleepResponse,
        hrv_avg: Option<f64>,
    ) -> Option<Sleep> {
        let dto = resp.daily_sleep_dto.as_ref()?;

        // Skip all-zero records — Garmin returns an empty DTO on days with no sleep data
        let has_data = dto.sleep_time_in_seconds.unwrap_or(0) > 0
            || dto.deep_sleep_seconds.unwrap_or(0) > 0
            || dto.rem_sleep_in_seconds.unwrap_or(0) > 0
            || dto.light_sleep_seconds.unwrap_or(0) > 0;
        if !has_data {
            return None;
        }

        Some(Sleep {
            date: date.to_string(),
            total_seconds: dto.sleep_time_in_seconds.unwrap_or_else(|| {
                dto.deep_sleep_seconds.unwrap_or(0)
                    + dto.rem_sleep_in_seconds.unwrap_or(0)
                    + dto.light_sleep_seconds.unwrap_or(0)
                    + dto.awake_sleep_seconds.unwrap_or(0)
            }),
            deep_seconds: dto.deep_sleep_seconds.unwrap_or(0),
            rem_seconds: dto.rem_sleep_in_seconds.unwrap_or(0),
            light_seconds: dto.light_sleep_seconds.unwrap_or(0),
            awake_seconds: dto.awake_sleep_seconds.unwrap_or(0),
            sleep_score: dto
                .sleep_scores
                .as_ref()
                .and_then(|s| s.overall.as_ref())
                .and_then(|o| o.value),
            hrv_ms: hrv_avg,
            start_time: Self::local_datetime(dto.sleep_start_timestamp_local),
            end_time: Self::local_datetime(dto.sleep_end_timestamp_local),
            spo2_avg: dto.average_sp_o2_value,
            respiration_avg: dto.average_respiration_value,
            source: "garmin".into(),
        })
    }

    /// `YYYY-MM-DD HH:MM:SS` for a Garmin `*TimestampLocal` value. Garmin sends epoch milliseconds
    /// already shifted to the device's local wall-clock time, so the UTC rendering of it is the
    /// local time. Integer division drops the milliseconds, as `datetime(ms / 1000, 'unixepoch')`
    /// does in migration v7; `local_datetime_matches_sqlite_datetime` pins the two together.
    fn local_datetime(timestamp_ms: Option<i64>) -> Option<String> {
        // Not `from_timestamp_millis`: it floors negative values, while SQLite truncates them.
        let datetime = chrono::DateTime::from_timestamp(timestamp_ms? / 1000, 0)?;
        Some(datetime.format("%Y-%m-%d %H:%M:%S").to_string())
    }

    fn map_heart(
        date: NaiveDate,
        summary: &GarminDailySummaryResponse,
        hrv_avg: Option<f64>,
        vo2_max: Option<f64>,
    ) -> Heart {
        Heart {
            date: date.to_string(),
            resting_hr: summary.resting_heart_rate,
            max_hr: summary.max_heart_rate,
            min_hr: summary.min_heart_rate,
            hrv_avg,
            vo2_max,
            source: "garmin".into(),
        }
    }

    fn map_recovery(
        date: NaiveDate,
        summary: &GarminDailySummaryResponse,
        body_battery_charged: Option<i32>,
        body_battery_drained: Option<i32>,
    ) -> Recovery {
        Recovery {
            date: date.to_string(),
            body_battery_charged,
            body_battery_drained,
            body_battery_peak: summary.body_battery_highest_value,
            body_battery_low: summary.body_battery_lowest_value,
            source: "garmin".into(),
        }
    }

    fn map_activity(date: NaiveDate, summary: &GarminDailySummaryResponse) -> Activity {
        let active_minutes = summary
            .active_seconds
            .or(summary.highly_active_seconds)
            .map(|s| (s / 60) as i32);

        Activity {
            date: date.to_string(),
            steps: summary.total_steps,
            active_minutes,
            floors: summary.floors_ascended.map(|f| f as i32),
            total_kcal: summary.total_kilocalories,
            active_kcal: summary.active_kilocalories,
            bmr_kcal: summary.bmr_kilocalories,
            distance_meters: summary.total_distance_meters,
            source: "garmin".into(),
        }
    }

    fn map_stress(date: NaiveDate, summary: &GarminDailySummaryResponse) -> Stress {
        Stress {
            date: date.to_string(),
            avg_stress: summary.average_stress_level,
            max_stress: summary.max_stress_level,
            rest_seconds: summary.rest_stress_duration,
            low_seconds: summary.low_stress_duration,
            medium_seconds: summary.medium_stress_duration,
            high_seconds: summary.high_stress_duration,
            source: "garmin".into(),
        }
    }
}

impl Default for GarminProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for GarminProvider {
    fn name(&self) -> &str {
        "garmin"
    }

    fn sync(&self, db: &Database, range: &DateRange) -> Result<SyncReport> {
        if !tokens::tokens_exist() {
            anyhow::bail!("Garmin tokens not found. Run `pulse garmin-login` first.");
        }

        let access_token = auth::ensure_valid_token(&self.client)
            .context("Failed to obtain valid Garmin access token")?;

        let api = GarminApi::new(&self.client, access_token);

        let mut records_synced: usize = 0;
        let mut errors: Vec<String> = Vec::new();
        let mut current = range.start;

        while current <= range.end {
            match self.sync_date(db, &api, current) {
                Ok(count) => records_synced += count,
                Err(e) => errors.push(format!("{current}: {e}")),
            }

            current += chrono::Duration::days(1);

            // Rate limiting: 200ms between dates
            if current <= range.end {
                thread::sleep(Duration::from_millis(200));
            }
        }

        // Record sync timestamp
        let now = chrono::Local::now().to_rfc3339();
        let _ = queries::set_sync_state(db, "garmin_last_sync", &now);

        Ok(SyncReport {
            provider: "garmin".into(),
            records_synced,
            errors,
        })
    }

    fn supported_metrics(&self) -> &[MetricType] {
        &[
            MetricType::Sleep,
            MetricType::Heart,
            MetricType::Recovery,
            MetricType::Activity,
            MetricType::Stress,
        ]
    }
}

/// The rows one daily-summary body maps to.
struct DailyRows {
    heart: Heart,
    recovery: Recovery,
    activity: Activity,
    stress: Stress,
}

impl GarminProvider {
    /// Sync all metrics for a single date. Returns count of records upserted.
    ///
    /// Each body is recorded in `raw_responses` before it is mapped, so a mapped row is never
    /// newer than its stored body.
    fn sync_date(&self, db: &Database, api: &GarminApi, date: NaiveDate) -> Result<usize> {
        let mut count = 0;
        let key = date.to_string();

        // 1. HRV (separate endpoint) — fetched first so sleep can include it
        let hrv_body = match api.fetch_body(GarminEndpoint::Hrv, &key) {
            Ok(body) => {
                self.record(db, GarminEndpoint::Hrv, &key, &body)?;
                Some(body)
            }
            Err(_) => None, // HRV often missing, don't fail the whole date
        };
        let hrv_avg = Self::hrv_avg_from_body(hrv_body.as_deref());

        // 2. Sleep
        let sleep_body = api.fetch_body(GarminEndpoint::Sleep, &key)?;
        self.record(db, GarminEndpoint::Sleep, &key, &sleep_body)?;
        if let Some(sleep) = Self::sleep_from_body(date, &sleep_body, hrv_avg)? {
            queries::upsert_sleep(db, &sleep)?;
            count += 1;
        }

        // 3. Daily summary → Heart, Recovery, Activity, Stress
        let summary_body = api.fetch_body(GarminEndpoint::DailySummary, &key)?;
        self.record(db, GarminEndpoint::DailySummary, &key, &summary_body)?;

        // 4. Body battery charged/drained (separate endpoint)
        let body_battery = match api.fetch_body(GarminEndpoint::BodyBattery, &key) {
            Ok(body) => {
                self.record(db, GarminEndpoint::BodyBattery, &key, &body)?;
                Self::body_battery_from_body(&body)
            }
            Err(e) => Err(e),
        }
        .unwrap_or_else(|e| {
            eprintln!("[garmin] body battery fetch failed for {date}: {e:#}");
            (None, None)
        });

        // 5. VO2 max (separate endpoint) — only days with a new estimate have one
        let vo2_max = match api.fetch_body(GarminEndpoint::MaxMet, &key) {
            Ok(body) => {
                self.record(db, GarminEndpoint::MaxMet, &key, &body)?;
                Self::vo2_max_from_body(date, &body)
            }
            Err(e) => Err(e),
        }
        .unwrap_or_else(|e| {
            eprintln!("[garmin] VO2 max fetch failed for {date}: {e:#}");
            None
        });

        let rows = Self::daily_from_body(date, &summary_body, hrv_avg, vo2_max, body_battery)?;
        count += Self::upsert_daily(db, &rows)?;

        Ok(count)
    }

    /// Record a received body in `raw_responses` (see [`record_raw_response`]).
    fn record(&self, db: &Database, endpoint: GarminEndpoint, key: &str, body: &str) -> Result<()> {
        record_raw_response(
            db,
            self.raw_responses,
            api::PROVIDER,
            endpoint.name(),
            key,
            body,
        )
    }

    /// Last-night HRV average from an HRV body. `None` when there is no body or it does not parse.
    fn hrv_avg_from_body(body: Option<&str>) -> Option<f64> {
        let resp = api::parse_body::<GarminHrvResponse>(GarminEndpoint::Hrv, body?).ok()?;
        resp.hrv_summary.and_then(|s| s.last_night_avg)
    }

    /// `(charged, drained)` from a Body Battery body. The first item wins.
    fn body_battery_from_body(body: &str) -> Result<(Option<i32>, Option<i32>)> {
        let items: GarminBodyBatteryResponse = api::parse_body(GarminEndpoint::BodyBattery, body)?;
        Ok(items
            .into_iter()
            .next()
            .map(|item| (item.charged, item.drained))
            .unwrap_or((None, None)))
    }

    /// VO2 max estimated on `date`, from a maxmet body: the `generic` entry whose `calendarDate`
    /// is `date`, `vo2MaxPreciseValue` first, then `vo2MaxValue`; a value of 0 or less is absent.
    /// `Ok(None)` for an empty body, `null`, `[]`, a cycling-only day, or entries for other dates.
    /// `Err` when the body is not a list of entries of the expected shape.
    fn vo2_max_from_body(date: NaiveDate, body: &str) -> Result<Option<f64>> {
        if body.trim().is_empty() {
            return Ok(None);
        }
        let entries: Option<GarminMaxMetResponse> = api::parse_body(GarminEndpoint::MaxMet, body)?;
        let date = date.to_string();
        Ok(entries
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.generic)
            .find(|estimate| estimate.calendar_date == date)
            .and_then(|estimate| estimate.vo2_max_precise_value.or(estimate.vo2_max_value))
            .filter(|value| *value > 0.0))
    }

    /// Map a sleep body to a [`Sleep`] row, or `None` when the day has no sleep data.
    fn sleep_from_body(date: NaiveDate, body: &str, hrv_avg: Option<f64>) -> Result<Option<Sleep>> {
        let resp: GarminSleepResponse = api::parse_body(GarminEndpoint::Sleep, body)?;
        Ok(Self::map_sleep(date, &resp, hrv_avg))
    }

    /// Map a daily-summary body to the heart, recovery, activity and stress rows.
    fn daily_from_body(
        date: NaiveDate,
        body: &str,
        hrv_avg: Option<f64>,
        vo2_max: Option<f64>,
        body_battery: (Option<i32>, Option<i32>),
    ) -> Result<DailyRows> {
        let summary: GarminDailySummaryResponse =
            api::parse_body(GarminEndpoint::DailySummary, body)?;
        Ok(DailyRows {
            heart: Self::map_heart(date, &summary, hrv_avg, vo2_max),
            recovery: Self::map_recovery(date, &summary, body_battery.0, body_battery.1),
            activity: Self::map_activity(date, &summary),
            stress: Self::map_stress(date, &summary),
        })
    }

    /// Write the four daily rows. Returns the number of records written.
    fn upsert_daily(db: &Database, rows: &DailyRows) -> Result<usize> {
        queries::upsert_heart(db, &rows.heart)?;
        queries::upsert_recovery(db, &rows.recovery)?;
        queries::upsert_activity(db, &rows.activity)?;
        queries::upsert_stress(db, &rows.stress)?;
        Ok(4)
    }
}

/// Refill sleep, heart, recovery, activity and stress from the Garmin bodies in `raw_responses`.
/// No network. Dates without a stored sleep or daily-summary body are left as they are.
/// A body that does not parse is reported in `errors`; a database error is returned.
pub(crate) fn rebuild(db: &Database) -> Result<SyncReport> {
    let stored = |endpoint: GarminEndpoint, key: &str| {
        queries::get_raw_response(db, api::PROVIDER, endpoint.name(), key)
    };

    let mut keys = BTreeSet::new();
    for endpoint in [GarminEndpoint::Sleep, GarminEndpoint::DailySummary] {
        keys.extend(queries::raw_response_keys(
            db,
            api::PROVIDER,
            endpoint.name(),
        )?);
    }

    let mut records_synced = 0;
    let mut errors = Vec::new();

    for key in keys {
        let Ok(date) = NaiveDate::parse_from_str(&key, "%Y-%m-%d") else {
            errors.push(format!("{key}: not a YYYY-MM-DD date"));
            continue;
        };

        let hrv_body = stored(GarminEndpoint::Hrv, &key)?;
        let hrv_avg = GarminProvider::hrv_avg_from_body(hrv_body.as_deref());

        if let Some(body) = stored(GarminEndpoint::Sleep, &key)? {
            match GarminProvider::sleep_from_body(date, &body, hrv_avg) {
                Ok(Some(sleep)) => {
                    queries::upsert_sleep(db, &sleep)?;
                    records_synced += 1;
                }
                Ok(None) => {}
                Err(e) => errors.push(format!("{key} sleep: {e:#}")),
            }
        }

        if let Some(body) = stored(GarminEndpoint::DailySummary, &key)? {
            let body_battery = match stored(GarminEndpoint::BodyBattery, &key)? {
                Some(bb_body) => {
                    GarminProvider::body_battery_from_body(&bb_body).unwrap_or_else(|e| {
                        errors.push(format!("{key} body battery: {e:#}"));
                        (None, None)
                    })
                }
                None => (None, None),
            };
            let vo2_max = match stored(GarminEndpoint::MaxMet, &key)? {
                Some(mm_body) => {
                    GarminProvider::vo2_max_from_body(date, &mm_body).unwrap_or_else(|e| {
                        errors.push(format!("{key} vo2 max: {e:#}"));
                        None
                    })
                }
                None => None,
            };
            match GarminProvider::daily_from_body(date, &body, hrv_avg, vo2_max, body_battery) {
                Ok(rows) => records_synced += GarminProvider::upsert_daily(db, &rows)?,
                Err(e) => errors.push(format!("{key} daily summary: {e:#}")),
            }
        }
    }

    Ok(SyncReport {
        provider: "garmin".into(),
        records_synced,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{
        DailySleepDto, GarminBodyBatteryItem, GarminDailySummaryResponse, GarminSleepResponse,
        SleepScoreValue, SleepScores,
    };

    #[test]
    fn provider_name() {
        let provider = GarminProvider::new();
        assert_eq!(provider.name(), "garmin");
    }

    #[test]
    fn supported_metrics_covers_all_health_data() {
        let provider = GarminProvider::new();
        let metrics = provider.supported_metrics();
        assert_eq!(metrics.len(), 5);
    }

    #[test]
    fn default_trait_works() {
        let provider = GarminProvider::default();
        assert_eq!(provider.name(), "garmin");
    }

    #[test]
    fn map_sleep_full_response() {
        let resp = GarminSleepResponse {
            daily_sleep_dto: Some(DailySleepDto {
                sleep_time_in_seconds: Some(28800),
                deep_sleep_seconds: Some(7200),
                rem_sleep_in_seconds: Some(5400),
                light_sleep_seconds: Some(14400),
                awake_sleep_seconds: Some(1800),
                sleep_start_timestamp_local: Some(1772320200999),
                sleep_end_timestamp_local: Some(1772348100000),
                average_sp_o2_value: Some(96.0),
                average_respiration_value: Some(14.5),
                sleep_scores: Some(SleepScores {
                    overall: Some(SleepScoreValue { value: Some(82) }),
                }),
            }),
        };
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let sleep = GarminProvider::map_sleep(date, &resp, Some(45.0)).unwrap();
        assert_eq!(sleep.date, "2026-03-01");
        assert_eq!(sleep.total_seconds, 28800);
        assert_eq!(sleep.deep_seconds, 7200);
        assert_eq!(sleep.sleep_score, Some(82));
        assert_eq!(sleep.hrv_ms, Some(45.0));
        assert_eq!(sleep.start_time.as_deref(), Some("2026-02-28 23:10:00"));
        assert_eq!(sleep.end_time.as_deref(), Some("2026-03-01 06:55:00"));
        assert_eq!(sleep.spo2_avg, Some(96.0));
        assert_eq!(sleep.respiration_avg, Some(14.5));
        assert_eq!(sleep.source, "garmin");
    }

    #[test]
    fn map_sleep_total_seconds_computed_from_phases_when_missing() {
        let resp = GarminSleepResponse {
            daily_sleep_dto: Some(DailySleepDto {
                sleep_time_in_seconds: None, // Garmin omits this sometimes
                deep_sleep_seconds: Some(7200),
                rem_sleep_in_seconds: Some(5400),
                light_sleep_seconds: Some(14400),
                awake_sleep_seconds: Some(1800),
                sleep_start_timestamp_local: None,
                sleep_end_timestamp_local: None,
                average_sp_o2_value: None,
                average_respiration_value: None,
                sleep_scores: None,
            }),
        };
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let sleep = GarminProvider::map_sleep(date, &resp, None).unwrap();
        assert_eq!(sleep.total_seconds, 7200 + 5400 + 14400 + 1800); // 28800
        assert_eq!(sleep.rem_seconds, 5400);
    }

    #[test]
    fn map_sleep_returns_none_when_dto_missing() {
        let resp = GarminSleepResponse {
            daily_sleep_dto: None,
        };
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        assert!(GarminProvider::map_sleep(date, &resp, None).is_none());
    }

    #[test]
    fn map_heart_with_hrv() {
        let summary = test_daily_summary();
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let heart = GarminProvider::map_heart(date, &summary, Some(42.5), None);
        assert_eq!(heart.resting_hr, Some(55));
        assert_eq!(heart.max_hr, Some(142));
        assert_eq!(heart.min_hr, Some(48));
        assert_eq!(heart.hrv_avg, Some(42.5));
    }

    #[test]
    fn map_heart_without_hrv() {
        let summary = test_daily_summary();
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let heart = GarminProvider::map_heart(date, &summary, None, None);
        assert!(heart.hrv_avg.is_none());
    }

    #[test]
    fn map_heart_takes_vo2_max_from_its_argument_not_the_summary() {
        let mut summary = test_daily_summary();
        summary.vo2_max_value = Some(48.5);

        let heart = GarminProvider::map_heart(day(), &summary, Some(42.5), Some(47.6));
        assert_eq!(heart.vo2_max, Some(47.6));
        assert_eq!(heart.hrv_avg, Some(42.5));

        let heart = GarminProvider::map_heart(day(), &summary, None, None);
        assert_eq!(heart.vo2_max, None);
    }

    #[test]
    fn map_recovery_from_summary() {
        let summary = test_daily_summary();
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let recovery = GarminProvider::map_recovery(date, &summary, None, None);
        assert_eq!(recovery.body_battery_peak, Some(95));
        assert_eq!(recovery.body_battery_low, Some(25));
        assert!(recovery.body_battery_charged.is_none());
        assert!(recovery.body_battery_drained.is_none());
    }

    #[test]
    fn map_recovery_with_charged_drained() {
        let summary = test_daily_summary();
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let recovery = GarminProvider::map_recovery(date, &summary, Some(85), Some(62));
        assert_eq!(recovery.body_battery_peak, Some(95));
        assert_eq!(recovery.body_battery_low, Some(25));
        assert_eq!(recovery.body_battery_charged, Some(85));
        assert_eq!(recovery.body_battery_drained, Some(62));
        assert_eq!(recovery.source, "garmin");
    }

    #[test]
    fn body_battery_item_first_wins() {
        // Simulate what sync_date does: take first item from the Vec
        let items = vec![
            GarminBodyBatteryItem {
                charged: Some(85),
                drained: Some(62),
            },
            GarminBodyBatteryItem {
                charged: Some(10),
                drained: Some(5),
            },
        ];
        let (charged, drained) = items
            .into_iter()
            .next()
            .map(|item| (item.charged, item.drained))
            .unwrap_or((None, None));
        assert_eq!(charged, Some(85));
        assert_eq!(drained, Some(62));
    }

    #[test]
    fn map_activity_from_summary() {
        let summary = test_daily_summary();
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let activity = GarminProvider::map_activity(date, &summary);
        assert_eq!(activity.steps, Some(12345));
        assert_eq!(activity.active_minutes, Some(60)); // 3600s / 60
        assert_eq!(activity.floors, Some(10));
        assert_eq!(activity.total_kcal, Some(2450.0));
        assert_eq!(activity.active_kcal, Some(612.0));
        assert_eq!(activity.bmr_kcal, Some(1838.0));
        assert_eq!(activity.distance_meters, Some(8420.0));
    }

    #[test]
    fn map_stress_from_summary() {
        let summary = test_daily_summary();
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let stress = GarminProvider::map_stress(date, &summary);
        assert_eq!(stress.avg_stress, Some(35));
        assert_eq!(stress.max_stress, Some(78));
        assert_eq!(stress.rest_seconds, Some(28800));
        assert_eq!(stress.low_seconds, Some(18000));
        assert_eq!(stress.medium_seconds, Some(4200));
        assert_eq!(stress.high_seconds, Some(600));
    }

    fn test_daily_summary() -> GarminDailySummaryResponse {
        GarminDailySummaryResponse {
            resting_heart_rate: Some(55),
            max_heart_rate: Some(142),
            min_heart_rate: Some(48),
            total_steps: Some(12345),
            highly_active_seconds: Some(1800),
            active_seconds: Some(3600),
            floors_ascended: Some(10.0),
            average_stress_level: Some(35),
            max_stress_level: Some(78),
            rest_stress_duration: Some(28800),
            low_stress_duration: Some(18000),
            medium_stress_duration: Some(4200),
            high_stress_duration: Some(600),
            body_battery_highest_value: Some(95),
            body_battery_lowest_value: Some(25),
            total_kilocalories: Some(2450.0),
            active_kilocalories: Some(612.0),
            bmr_kilocalories: Some(1838.0),
            total_distance_meters: Some(8420.0),
            vo2_max_value: None,
        }
    }

    const DAY: &str = "2026-03-01";
    const SLEEP_BODY: &str = r#"{"dailySleepDTO":{"sleepTimeInSeconds":28800,"deepSleepSeconds":7200,"remSleepSeconds":5400,"lightSleepSeconds":14400,"awakeSleepSeconds":1800,"sleepStartTimestampLocal":1772320200999,"sleepEndTimestampLocal":1772348100000,"averageSpO2Value":95.0,"averageRespirationValue":14.5,"sleepScores":{"overall":{"value":82}}},"sleepMovement":[{"activityLevel":0.5}]}"#;
    const HRV_BODY: &str = r#"{"hrvSummary":{"lastNightAvg":42.5}}"#;
    const SUMMARY_BODY: &str = r#"{"restingHeartRate":55,"maxHeartRate":142,"minHeartRate":48,"totalSteps":12345,"activeSeconds":3600,"floorsAscended":10,"averageStressLevel":35,"maxStressLevel":78,"bodyBatteryHighestValue":95,"bodyBatteryLowestValue":25,"vo2MaxValue":48.5,"totalKilocalories":2450.0,"activeKilocalories":612.0,"bmrKilocalories":1838.0,"totalDistanceMeters":8420,"restStressDuration":28800,"lowStressDuration":18000,"mediumStressDuration":4200,"highStressDuration":null}"#;
    const BB_BODY: &str = r#"[{"charged":85,"drained":62}]"#;
    /// Modeled on public maxmet samples (docs/plans/2026-10-16-vo2-max.md); not yet checked
    /// against a live response from the owner's account.
    const MAXMET_BODY: &str = r#"[{"userId":12345678,"generic":{"calendarDate":"2026-03-01","vo2MaxPreciseValue":47.6,"vo2MaxValue":48.0,"fitnessAge":null,"fitnessAgeDescription":null,"maxMetCategory":0},"cycling":null,"heatAltitudeAcclimation":{"acclimationPercentage":0,"altitudeAcclimation":0,"heatAcclimationPercentage":0}}]"#;
    const MAXMET_CYCLING_ONLY_BODY: &str = r#"[{"userId":1,"generic":null,"cycling":{"calendarDate":"2026-03-01","vo2MaxPreciseValue":55.2,"vo2MaxValue":55,"maxMetCategory":1}}]"#;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
    }

    fn store(db: &Database, endpoint: GarminEndpoint, key: &str, body: &str) {
        queries::upsert_raw_response(db, api::PROVIDER, endpoint.name(), key, body).unwrap();
    }

    /// Number of rows in `table` for `date`.
    fn rows_on(db: &Database, table: &str, date: &str) -> i64 {
        db.conn()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE date = ?1"),
                [date],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn with_raw_responses_sets_the_switch() {
        assert!(GarminProvider::new().raw_responses);
        assert!(
            !GarminProvider::new()
                .with_raw_responses(false)
                .raw_responses
        );
    }

    #[test]
    fn hrv_avg_from_body_is_none_when_absent_or_unparseable() {
        assert_eq!(GarminProvider::hrv_avg_from_body(None), None);
        assert_eq!(GarminProvider::hrv_avg_from_body(Some("")), None);
        assert_eq!(GarminProvider::hrv_avg_from_body(Some("[1]")), None);
        assert_eq!(
            GarminProvider::hrv_avg_from_body(Some(HRV_BODY)),
            Some(42.5)
        );
    }

    #[test]
    fn sleep_from_body_maps_and_takes_hrv() {
        let sleep = GarminProvider::sleep_from_body(day(), SLEEP_BODY, Some(42.5))
            .unwrap()
            .unwrap();
        assert_eq!(sleep.sleep_score, Some(82));
        assert_eq!(sleep.hrv_ms, Some(42.5));
        assert_eq!(sleep.total_seconds, 28800);
    }

    #[test]
    fn sleep_from_body_is_none_without_sleep_data() {
        let sleep =
            GarminProvider::sleep_from_body(day(), r#"{"dailySleepDTO":null}"#, None).unwrap();
        assert!(sleep.is_none());
    }

    #[test]
    fn sleep_from_body_errors_on_a_body_of_the_wrong_shape() {
        assert!(GarminProvider::sleep_from_body(day(), r#"{"dailySleepDTO":5}"#, None).is_err());
    }

    #[test]
    fn vo2_max_from_body_prefers_the_precise_value() {
        assert_eq!(
            GarminProvider::vo2_max_from_body(day(), MAXMET_BODY).unwrap(),
            Some(47.6)
        );
    }

    #[test]
    fn vo2_max_from_body_falls_back_to_the_rounded_value() {
        let body = r#"[{"generic":{"calendarDate":"2026-03-01","vo2MaxPreciseValue":null,"vo2MaxValue":48}}]"#;
        assert_eq!(
            GarminProvider::vo2_max_from_body(day(), body).unwrap(),
            Some(48.0)
        );
    }

    #[test]
    fn vo2_max_from_body_is_none_without_an_estimate() {
        let zero =
            r#"[{"generic":{"calendarDate":"2026-03-01","vo2MaxPreciseValue":0,"vo2MaxValue":0}}]"#;
        for body in ["", "  ", "null", "[]", MAXMET_CYCLING_ONLY_BODY, zero] {
            assert_eq!(
                GarminProvider::vo2_max_from_body(day(), body).unwrap(),
                None,
                "{body:?}"
            );
        }
    }

    #[test]
    fn vo2_max_from_body_takes_only_the_requested_date() {
        let earlier = r#"{"generic":{"calendarDate":"2026-02-27","vo2MaxPreciseValue":46.1}}"#;
        let requested = r#"{"generic":{"calendarDate":"2026-03-01","vo2MaxPreciseValue":47.6}}"#;
        assert_eq!(
            GarminProvider::vo2_max_from_body(day(), &format!("[{earlier},{requested}]")).unwrap(),
            Some(47.6)
        );
        assert_eq!(
            GarminProvider::vo2_max_from_body(day(), &format!("[{earlier}]")).unwrap(),
            None
        );
    }

    #[test]
    fn vo2_max_from_body_errors_on_an_unexpected_shape() {
        let err = GarminProvider::vo2_max_from_body(
            day(),
            r#"{"generic":{"calendarDate":"2026-03-01"}}"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("Failed to parse VO2 max response"));
        for body in [
            r#"[{"generic":{"vo2MaxValue":48}}]"#,
            r#"[{"generic":{"calendarDate":"2026-03-01","vo2MaxValue":"high"}}]"#,
        ] {
            assert!(
                GarminProvider::vo2_max_from_body(day(), body).is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn daily_from_body_maps_all_four_rows() {
        let rows = GarminProvider::daily_from_body(
            day(),
            SUMMARY_BODY,
            Some(42.5),
            Some(47.6),
            (Some(85), Some(62)),
        )
        .unwrap();
        assert_eq!(rows.heart.resting_hr, Some(55));
        assert_eq!(rows.heart.hrv_avg, Some(42.5));
        assert_eq!(rows.heart.vo2_max, Some(47.6));
        assert_eq!(rows.recovery.body_battery_peak, Some(95));
        assert_eq!(rows.recovery.body_battery_charged, Some(85));
        assert_eq!(rows.activity.steps, Some(12345));
        assert_eq!(rows.stress.max_stress, Some(78));
    }

    #[test]
    fn daily_from_body_ignores_vo2_max_in_the_summary() {
        // SUMMARY_BODY carries "vo2MaxValue":48.5; pulse no longer reads it.
        let rows =
            GarminProvider::daily_from_body(day(), SUMMARY_BODY, None, None, (None, None)).unwrap();
        assert_eq!(rows.heart.vo2_max, None);
    }

    /// `heart.vo2_max` for `date`, `None` when the row has no value.
    fn heart_vo2_max(db: &Database, date: &str) -> Option<f64> {
        db.conn()
            .query_row("SELECT vo2_max FROM heart WHERE date = ?1", [date], |r| {
                r.get(0)
            })
            .unwrap()
    }

    #[test]
    fn rebuild_leaves_vo2_max_null_without_an_estimate() {
        let db = Database::open_memory().unwrap();
        store(&db, GarminEndpoint::DailySummary, DAY, SUMMARY_BODY);
        store(
            &db,
            GarminEndpoint::DailySummary,
            "2026-03-02",
            SUMMARY_BODY,
        );
        store(&db, GarminEndpoint::MaxMet, "2026-03-02", "[]");

        let report = rebuild(&db).unwrap();

        assert!(report.errors.is_empty());
        for date in [DAY, "2026-03-02"] {
            assert_eq!(rows_on(&db, "heart", date), 1, "{date}");
            let resting_hr: Option<i32> = db
                .conn()
                .query_row(
                    "SELECT resting_hr FROM heart WHERE date = ?1",
                    [date],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(resting_hr, Some(55), "{date}");
            assert_eq!(heart_vo2_max(&db, date), None, "{date}");
        }
    }

    #[test]
    fn rebuild_reports_a_maxmet_body_that_does_not_parse_and_keeps_the_day() {
        let db = Database::open_memory().unwrap();
        store(&db, GarminEndpoint::DailySummary, DAY, SUMMARY_BODY);
        store(&db, GarminEndpoint::MaxMet, DAY, r#"{"generic":{}}"#);

        let report = rebuild(&db).unwrap();

        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].starts_with("2026-03-01 vo2 max:"));
        assert_eq!(report.records_synced, 4);
        assert_eq!(rows_on(&db, "heart", DAY), 1);
        assert_eq!(heart_vo2_max(&db, DAY), None);
    }

    #[test]
    fn rebuild_refills_the_daily_tables_from_stored_bodies() {
        let db = Database::open_memory().unwrap();
        store(&db, GarminEndpoint::Sleep, DAY, SLEEP_BODY);
        store(&db, GarminEndpoint::Hrv, DAY, HRV_BODY);
        store(&db, GarminEndpoint::DailySummary, DAY, SUMMARY_BODY);
        store(&db, GarminEndpoint::BodyBattery, DAY, BB_BODY);
        store(&db, GarminEndpoint::MaxMet, DAY, MAXMET_BODY);

        let report = rebuild(&db).unwrap();

        assert_eq!(report.provider, "garmin");
        assert_eq!(report.records_synced, 5);
        assert!(report.errors.is_empty());
        for table in ["sleep", "heart", "recovery", "activity", "stress"] {
            assert_eq!(rows_on(&db, table, DAY), 1, "{table}");
        }
        let sleep = queries::get_sleep(&db, DAY).unwrap().unwrap();
        assert_eq!(sleep.hrv_ms, Some(42.5));
        assert_eq!(sleep.start_time.as_deref(), Some("2026-02-28 23:10:00"));
        assert_eq!(sleep.respiration_avg, Some(14.5));
        let rest_seconds: Option<i64> = db
            .conn()
            .query_row(
                "SELECT rest_seconds FROM stress WHERE date = ?1",
                [DAY],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rest_seconds, Some(28800));
        let total_kcal: Option<f64> = db
            .conn()
            .query_row(
                "SELECT total_kcal FROM activity WHERE date = ?1",
                [DAY],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(total_kcal, Some(2450.0));
        assert_eq!(heart_vo2_max(&db, DAY), Some(47.6));
        let drained: Option<i32> = db
            .conn()
            .query_row(
                "SELECT body_battery_drained FROM recovery WHERE date = ?1",
                [DAY],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(drained, Some(62));
    }

    #[test]
    fn rebuild_replaces_rows_from_the_stored_body() {
        let db = Database::open_memory().unwrap();
        let mut sleep = GarminProvider::sleep_from_body(day(), SLEEP_BODY, None)
            .unwrap()
            .unwrap();
        sleep.sleep_score = Some(70);
        queries::upsert_sleep(&db, &sleep).unwrap();
        store(&db, GarminEndpoint::Sleep, DAY, SLEEP_BODY);

        rebuild(&db).unwrap();

        let rebuilt = queries::get_sleep(&db, DAY).unwrap().unwrap();
        assert_eq!(rebuilt.sleep_score, Some(82));
    }

    #[test]
    fn rebuild_leaves_days_without_a_body_alone() {
        let db = Database::open_memory().unwrap();
        let mut sleep = GarminProvider::sleep_from_body(day(), SLEEP_BODY, None)
            .unwrap()
            .unwrap();
        sleep.date = "2026-02-01".into();
        queries::upsert_sleep(&db, &sleep).unwrap();

        let report = rebuild(&db).unwrap();

        assert_eq!(report.records_synced, 0);
        let kept = queries::get_sleep(&db, "2026-02-01").unwrap().unwrap();
        assert_eq!(kept.sleep_score, Some(82));
    }

    #[test]
    fn rebuild_reports_a_body_that_does_not_parse_and_continues() {
        let db = Database::open_memory().unwrap();
        store(
            &db,
            GarminEndpoint::Sleep,
            "2026-03-02",
            r#"{"dailySleepDTO":5}"#,
        );
        store(&db, GarminEndpoint::Sleep, DAY, SLEEP_BODY);

        let report = rebuild(&db).unwrap();

        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].starts_with("2026-03-02 sleep:"));
        assert_eq!(rows_on(&db, "sleep", DAY), 1);
        assert_eq!(report.records_synced, 1);
    }

    #[test]
    fn local_datetime_matches_sqlite_datetime() {
        let db = Database::open_memory().unwrap();
        for ms in [
            Some(1792030930000_i64),
            Some(1792030930999),
            Some(1792050610000),
            Some(1772320200999),
            Some(0),
            Some(-1500),
            None,
        ] {
            let in_sql: Option<String> = db
                .conn()
                .query_row("SELECT datetime(?1 / 1000, 'unixepoch')", [ms], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(GarminProvider::local_datetime(ms), in_sql, "{ms:?}");
        }
        assert_eq!(
            GarminProvider::local_datetime(Some(1792030930999)),
            Some("2026-10-15 02:22:10".to_string())
        );
    }

    /// The v7 migration repeats the Garmin mapping in SQL; both must give the same values.
    #[test]
    fn migration_v7_backfill_matches_the_rust_mapping() {
        let db = Database::open_memory().unwrap();
        store(&db, GarminEndpoint::Sleep, DAY, SLEEP_BODY);
        store(&db, GarminEndpoint::DailySummary, DAY, SUMMARY_BODY);

        let sleep = GarminProvider::sleep_from_body(day(), SLEEP_BODY, None)
            .unwrap()
            .unwrap();
        let rows =
            GarminProvider::daily_from_body(day(), SUMMARY_BODY, None, None, (None, None)).unwrap();

        // Rows as 0.5 left them: the new columns are NULL.
        queries::upsert_sleep(
            &db,
            &Sleep {
                start_time: None,
                end_time: None,
                spo2_avg: None,
                respiration_avg: None,
                ..sleep.clone()
            },
        )
        .unwrap();
        queries::upsert_activity(
            &db,
            &Activity {
                total_kcal: None,
                active_kcal: None,
                bmr_kcal: None,
                distance_meters: None,
                ..rows.activity.clone()
            },
        )
        .unwrap();
        queries::upsert_stress(
            &db,
            &Stress {
                rest_seconds: None,
                low_seconds: None,
                medium_seconds: None,
                high_seconds: None,
                ..rows.stress.clone()
            },
        )
        .unwrap();

        crate::db::migrations::migrate_v7(&db).unwrap();

        // Raw SQL, because `query_*` only return recent days and DAY is in the past.
        let sleep_sql: (Option<String>, Option<String>, Option<f64>, Option<f64>) = db
            .conn()
            .query_row(
                "SELECT start_time, end_time, spo2_avg, respiration_avg FROM sleep WHERE date = ?1",
                [DAY],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        let stress_sql: (Option<i64>, Option<i64>, Option<i64>, Option<i64>) = db
            .conn()
            .query_row(
                "SELECT rest_seconds, low_seconds, medium_seconds, high_seconds FROM stress WHERE date = ?1",
                [DAY],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        let activity_sql: (Option<f64>, Option<f64>, Option<f64>, Option<f64>) = db
            .conn()
            .query_row(
                "SELECT total_kcal, active_kcal, bmr_kcal, distance_meters FROM activity WHERE date = ?1",
                [DAY],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();

        assert_eq!(
            sleep_sql,
            (
                sleep.start_time.clone(),
                sleep.end_time.clone(),
                sleep.spo2_avg,
                sleep.respiration_avg
            )
        );
        assert_eq!(
            stress_sql,
            (
                rows.stress.rest_seconds,
                rows.stress.low_seconds,
                rows.stress.medium_seconds,
                rows.stress.high_seconds
            )
        );
        assert_eq!(
            activity_sql,
            (
                rows.activity.total_kcal,
                rows.activity.active_kcal,
                rows.activity.bmr_kcal,
                rows.activity.distance_meters
            )
        );

        // Not vacuous: the values are present, and the edge cases hold.
        assert_eq!(sleep.start_time.as_deref(), Some("2026-02-28 23:10:00"));
        assert_eq!(rows.stress.high_seconds, None);
        assert_eq!(rows.activity.distance_meters, Some(8420.0));
    }
}
