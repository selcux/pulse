pub mod api;
pub mod auth;
pub mod tokens;

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::db::queries;
use crate::db::Database;
use crate::models::{Activity, Heart, Recovery, Sleep, Stress};
use crate::providers::{DateRange, MetricType, Provider, SyncReport};

use api::{GarminApi, GarminDailySummaryResponse, GarminSleepResponse};

pub struct GarminProvider {
    client: reqwest::blocking::Client,
}

impl GarminProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::ClientBuilder::new()
                .cookie_store(true)
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("Failed to build HTTP client"),
        }
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

    fn map_sleep(date: NaiveDate, resp: &GarminSleepResponse, hrv_avg: Option<f64>) -> Option<Sleep> {
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
            source: "garmin".into(),
        })
    }

    fn map_heart(
        date: NaiveDate,
        summary: &GarminDailySummaryResponse,
        hrv_avg: Option<f64>,
    ) -> Heart {
        Heart {
            date: date.to_string(),
            resting_hr: summary.resting_heart_rate,
            max_hr: summary.max_heart_rate,
            min_hr: summary.min_heart_rate,
            hrv_avg,
            vo2_max: summary.vo2_max_value,
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
            source: "garmin".into(),
        }
    }

    fn map_stress(date: NaiveDate, summary: &GarminDailySummaryResponse) -> Stress {
        Stress {
            date: date.to_string(),
            avg_stress: summary.average_stress_level,
            max_stress: summary.max_stress_level,
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
            anyhow::bail!(
                "Garmin tokens not found. Run `pulse garmin-login` first."
            );
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

impl GarminProvider {
    /// Sync all metrics for a single date. Returns count of records upserted.
    fn sync_date(&self, db: &Database, api: &GarminApi, date: NaiveDate) -> Result<usize> {
        let mut count = 0;

        // 1. HRV (separate endpoint) — fetched first so sleep can include it
        let hrv_avg = match api.fetch_hrv(date) {
            Ok(hrv_resp) => hrv_resp
                .hrv_summary
                .and_then(|s| s.last_night_avg),
            Err(_) => None, // HRV often missing, don't fail the whole date
        };

        // 2. Sleep
        let sleep_resp = api.fetch_sleep(date)?;
        if let Some(sleep) = Self::map_sleep(date, &sleep_resp, hrv_avg) {
            queries::upsert_sleep(db, &sleep)?;
            count += 1;
        }

        // 3. Daily summary → Heart, Recovery, Activity, Stress
        let summary = api.fetch_daily_summary(date)?;

        let heart = Self::map_heart(date, &summary, hrv_avg);
        queries::upsert_heart(db, &heart)?;
        count += 1;

        // 4. Body battery charged/drained (separate endpoint)
        let (bb_charged, bb_drained) = match api.fetch_body_battery(date) {
            Ok(items) => items
                .into_iter()
                .next()
                .map(|item| (item.charged, item.drained))
                .unwrap_or((None, None)),
            Err(e) => {
                eprintln!("[garmin] body battery fetch failed for {date}: {e:#}");
                (None, None)
            }
        };

        let recovery = Self::map_recovery(date, &summary, bb_charged, bb_drained);
        queries::upsert_recovery(db, &recovery)?;
        count += 1;

        let activity = Self::map_activity(date, &summary);
        queries::upsert_activity(db, &activity)?;
        count += 1;

        let stress = Self::map_stress(date, &summary);
        queries::upsert_stress(db, &stress)?;
        count += 1;

        Ok(count)
    }
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
                average_sp_o2_value: Some(96.0),
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
                average_sp_o2_value: None,
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
        let heart = GarminProvider::map_heart(date, &summary, Some(42.5));
        assert_eq!(heart.resting_hr, Some(55));
        assert_eq!(heart.max_hr, Some(142));
        assert_eq!(heart.min_hr, Some(48));
        assert_eq!(heart.hrv_avg, Some(42.5));
    }

    #[test]
    fn map_heart_without_hrv() {
        let summary = test_daily_summary();
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let heart = GarminProvider::map_heart(date, &summary, None);
        assert!(heart.hrv_avg.is_none());
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
            GarminBodyBatteryItem { charged: Some(85), drained: Some(62) },
            GarminBodyBatteryItem { charged: Some(10), drained: Some(5) },
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
    }

    #[test]
    fn map_stress_from_summary() {
        let summary = test_daily_summary();
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let stress = GarminProvider::map_stress(date, &summary);
        assert_eq!(stress.avg_stress, Some(35));
        assert_eq!(stress.max_stress, Some(78));
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
            body_battery_highest_value: Some(95),
            body_battery_lowest_value: Some(25),
            total_kilocalories: None,
            active_kilocalories: None,
            bmr_kilocalories: None,
            total_distance_meters: None,
            vo2_max_value: None,
        }
    }
}
