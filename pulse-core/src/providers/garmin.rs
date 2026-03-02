use std::path::PathBuf;

use anyhow::Result;
use chrono::NaiveDate;
use serde::Deserialize;

use crate::db::Database;
use crate::models::{Activity, Heart, Recovery, Sleep, Stress};
use crate::providers::{DateRange, MetricType, Provider, SyncReport};

// ---------------------------------------------------------------------------
// Garmin Connect API base URLs (documented for future implementation)
// ---------------------------------------------------------------------------
const _GARMIN_CONNECT_BASE: &str = "https://connect.garmin.com/modern/proxy";

// Endpoint templates:
// Sleep:        {base}/wellness-service/wellness/dailySleepData/{date}
// Heart:        {base}/userstats-service/stats/heartRate/daily/{date}/{date}
// Body Battery: {base}/device-service/usersummary?calendarDate={date}
// Steps:        {base}/device-service/usersummary?calendarDate={date}  (same endpoint)
// Stress:       {base}/userstats-service/stats/stress/daily/{date}/{date}

// ---------------------------------------------------------------------------
// API response shapes (private — ready for when auth is wired up)
// ---------------------------------------------------------------------------

/// Garmin daily sleep response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GarminSleepResponse {
    calendar_date: Option<String>,
    sleep_time_in_seconds: Option<i64>,
    deep_sleep_seconds: Option<i64>,
    rem_sleep_in_seconds: Option<i64>,
    light_sleep_seconds: Option<i64>,
    awake_sleep_seconds: Option<i64>,
    overall_score: Option<GarminSleepScore>,
    average_sp_o2_value: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GarminSleepScore {
    value: Option<i32>,
}

/// Garmin heart rate stats response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GarminHeartResponse {
    resting_heart_rate: Option<i32>,
    max_heart_rate: Option<i32>,
    min_heart_rate: Option<i32>,
}

/// Garmin user summary (body battery, steps, active minutes).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GarminUserSummary {
    total_steps: Option<i32>,
    highly_active_seconds: Option<i64>,
    active_seconds: Option<i64>,
    floors_ascended: Option<i32>,
    body_battery_charged_value: Option<i32>,
    body_battery_drained_value: Option<i32>,
    body_battery_highest_value: Option<i32>,
    body_battery_lowest_value: Option<i32>,
}

/// Garmin stress stats response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GarminStressResponse {
    overall_stress_level: Option<i32>,
    max_stress_level: Option<i32>,
}

// ---------------------------------------------------------------------------
// Provider implementation
// ---------------------------------------------------------------------------

pub struct GarminProvider {
    #[allow(dead_code)]
    client: reqwest::blocking::Client,
    #[allow(dead_code)]
    token_dir: PathBuf,
}

impl GarminProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
            token_dir: dirs::home_dir()
                .expect("Could not determine home directory")
                .join(".clawdbot/garmin"),
        }
    }

    // -----------------------------------------------------------------------
    // Stub fetch helpers — URL construction and response mapping ready,
    // auth headers not yet wired up.
    // -----------------------------------------------------------------------

    /// Fetch sleep data for a single date.
    #[allow(dead_code)]
    fn fetch_sleep(&self, date: NaiveDate) -> Result<Sleep> {
        let _url = format!(
            "{_GARMIN_CONNECT_BASE}/wellness-service/wellness/dailySleepData/{date}"
        );

        // TODO: Add auth headers from token_dir, make request, deserialize GarminSleepResponse
        // For now, bail with a clear message.
        anyhow::bail!("Garmin sleep fetch not yet implemented (auth required)")
    }

    /// Fetch heart rate data for a single date.
    #[allow(dead_code)]
    fn fetch_heart(&self, date: NaiveDate) -> Result<Heart> {
        let _url = format!(
            "{_GARMIN_CONNECT_BASE}/userstats-service/stats/heartRate/daily/{date}/{date}"
        );

        anyhow::bail!("Garmin heart fetch not yet implemented (auth required)")
    }

    /// Fetch body battery / recovery data for a single date.
    #[allow(dead_code)]
    fn fetch_recovery(&self, date: NaiveDate) -> Result<Recovery> {
        let _url = format!(
            "{_GARMIN_CONNECT_BASE}/device-service/usersummary?calendarDate={date}"
        );

        anyhow::bail!("Garmin recovery fetch not yet implemented (auth required)")
    }

    /// Fetch daily activity (steps, active minutes, floors) for a single date.
    #[allow(dead_code)]
    fn fetch_activity(&self, date: NaiveDate) -> Result<Activity> {
        // Same user summary endpoint as recovery
        let _url = format!(
            "{_GARMIN_CONNECT_BASE}/device-service/usersummary?calendarDate={date}"
        );

        anyhow::bail!("Garmin activity fetch not yet implemented (auth required)")
    }

    /// Fetch stress data for a single date.
    #[allow(dead_code)]
    fn fetch_stress(&self, date: NaiveDate) -> Result<Stress> {
        let _url = format!(
            "{_GARMIN_CONNECT_BASE}/userstats-service/stats/stress/daily/{date}/{date}"
        );

        anyhow::bail!("Garmin stress fetch not yet implemented (auth required)")
    }

    // -----------------------------------------------------------------------
    // Mapping helpers (ready for when API calls work)
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    fn map_sleep(date: NaiveDate, resp: &GarminSleepResponse) -> Sleep {
        Sleep {
            date: date.to_string(),
            total_seconds: resp.sleep_time_in_seconds.unwrap_or(0),
            deep_seconds: resp.deep_sleep_seconds.unwrap_or(0),
            rem_seconds: resp.rem_sleep_in_seconds.unwrap_or(0),
            light_seconds: resp.light_sleep_seconds.unwrap_or(0),
            awake_seconds: resp.awake_sleep_seconds.unwrap_or(0),
            sleep_score: resp.overall_score.as_ref().and_then(|s| s.value),
            hrv_ms: None, // Garmin HRV comes from a separate endpoint
            source: "garmin".into(),
        }
    }

    #[allow(dead_code)]
    fn map_heart(date: NaiveDate, resp: &GarminHeartResponse) -> Heart {
        Heart {
            date: date.to_string(),
            resting_hr: resp.resting_heart_rate,
            max_hr: resp.max_heart_rate,
            min_hr: resp.min_heart_rate,
            hrv_avg: None, // HRV not in this endpoint
            source: "garmin".into(),
        }
    }

    #[allow(dead_code)]
    fn map_recovery(date: NaiveDate, resp: &GarminUserSummary) -> Recovery {
        Recovery {
            date: date.to_string(),
            body_battery_charged: resp.body_battery_charged_value,
            body_battery_drained: resp.body_battery_drained_value,
            body_battery_peak: resp.body_battery_highest_value,
            body_battery_low: resp.body_battery_lowest_value,
            source: "garmin".into(),
        }
    }

    #[allow(dead_code)]
    fn map_activity(date: NaiveDate, resp: &GarminUserSummary) -> Activity {
        let active_minutes = resp
            .active_seconds
            .or(resp.highly_active_seconds)
            .map(|s| (s / 60) as i32);

        Activity {
            date: date.to_string(),
            steps: resp.total_steps,
            active_minutes,
            floors: resp.floors_ascended,
            source: "garmin".into(),
        }
    }

    #[allow(dead_code)]
    fn map_stress(date: NaiveDate, resp: &GarminStressResponse) -> Stress {
        Stress {
            date: date.to_string(),
            avg_stress: resp.overall_stress_level,
            max_stress: resp.max_stress_level,
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

    fn sync(&self, _db: &Database, _range: &DateRange) -> Result<SyncReport> {
        // Garmin Connect uses complex SSO auth (CSRF tokens, CloudFlare challenges).
        // The plan is to reuse python-garminconnect tokens from ~/.clawdbot/garmin/
        // once that auth bridge is built.
        //
        // Future implementation outline for each date in range:
        //   1. fetch_sleep(date)    -> queries::upsert_sleep(db, &sleep)
        //   2. fetch_heart(date)    -> queries::upsert_heart(db, &heart)
        //   3. fetch_recovery(date) -> queries::upsert_recovery(db, &recovery)
        //   4. fetch_activity(date) -> queries::upsert_activity(db, &activity)
        //   5. fetch_stress(date)   -> queries::upsert_stress(db, &stress)
        //   6. queries::set_sync_state(db, "garmin_last_sync", &now)

        anyhow::bail!(
            "Garmin provider: auth not yet implemented. \
             Use the Python `garmin` CLI tool for now."
        )
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn map_sleep_from_garmin_response() {
        let resp = GarminSleepResponse {
            calendar_date: Some("2026-03-01".into()),
            sleep_time_in_seconds: Some(28800),
            deep_sleep_seconds: Some(7200),
            rem_sleep_in_seconds: Some(5400),
            light_sleep_seconds: Some(14400),
            awake_sleep_seconds: Some(1800),
            overall_score: Some(GarminSleepScore { value: Some(82) }),
            average_sp_o2_value: Some(96.0),
        };
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let sleep = GarminProvider::map_sleep(date, &resp);
        assert_eq!(sleep.date, "2026-03-01");
        assert_eq!(sleep.total_seconds, 28800);
        assert_eq!(sleep.sleep_score, Some(82));
        assert_eq!(sleep.source, "garmin");
    }

    #[test]
    fn map_activity_from_garmin_summary() {
        let resp = GarminUserSummary {
            total_steps: Some(12345),
            highly_active_seconds: Some(1800),
            active_seconds: Some(3600),
            floors_ascended: Some(10),
            body_battery_charged_value: None,
            body_battery_drained_value: None,
            body_battery_highest_value: None,
            body_battery_lowest_value: None,
        };
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let activity = GarminProvider::map_activity(date, &resp);
        assert_eq!(activity.steps, Some(12345));
        assert_eq!(activity.active_minutes, Some(60)); // 3600s / 60
        assert_eq!(activity.floors, Some(10));
    }
}
