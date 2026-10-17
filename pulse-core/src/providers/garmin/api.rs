use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use serde::Deserialize;

const CONNECT_API: &str = "https://connectapi.garmin.com";

// ---------------------------------------------------------------------------
// Response types (matching actual Garmin Connect API shapes)
// ---------------------------------------------------------------------------

/// Exercise sets endpoint: /activity-service/activity/{id}/exerciseSets
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GarminExerciseSetsResponse {
    pub exercise_sets: Vec<GarminExerciseSet>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GarminExerciseSet {
    pub exercises: Vec<GarminExercise>,
    pub repetition_count: Option<i32>,
    /// Weight in grams (divide by 1000 to get kg).
    pub weight: Option<f64>,
    pub set_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GarminExercise {
    pub category: Option<String>,
    pub name: Option<String>,
}

/// Sleep endpoint: /sleep-service/sleep/dailySleepData?date={d}
#[derive(Debug, Deserialize)]
pub struct GarminSleepResponse {
    #[serde(rename = "dailySleepDTO")]
    pub daily_sleep_dto: Option<DailySleepDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailySleepDto {
    pub sleep_time_in_seconds: Option<i64>,
    pub deep_sleep_seconds: Option<i64>,
    #[serde(rename = "remSleepSeconds")]
    pub rem_sleep_in_seconds: Option<i64>,
    pub light_sleep_seconds: Option<i64>,
    pub awake_sleep_seconds: Option<i64>,
    /// Sleep start, in epoch milliseconds shifted to the device's local wall-clock time (not UTC).
    pub sleep_start_timestamp_local: Option<i64>,
    /// Sleep end (wake-up), in epoch milliseconds shifted to local wall-clock time (not UTC).
    pub sleep_end_timestamp_local: Option<i64>,
    pub average_sp_o2_value: Option<f64>,
    /// Average respiration rate during sleep, in breaths per minute.
    pub average_respiration_value: Option<f64>,
    pub sleep_scores: Option<SleepScores>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SleepScores {
    pub overall: Option<SleepScoreValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SleepScoreValue {
    pub value: Option<i32>,
}

/// HRV endpoint: /hrv-service/hrv/{d}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GarminHrvResponse {
    pub hrv_summary: Option<HrvSummary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HrvSummary {
    pub last_night_avg: Option<f64>,
}

/// Body battery: /wellness-service/wellness/dailyBatteryReport/display/{date}
/// Returns a list; we only care about the first (and usually only) item.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GarminBodyBatteryItem {
    pub charged: Option<i32>,
    pub drained: Option<i32>,
}

pub type GarminBodyBatteryResponse = Vec<GarminBodyBatteryItem>;

/// Daily summary: /usersummary-service/usersummary/daily/?calendarDate={d}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GarminDailySummaryResponse {
    // Heart rate fields - may not be in this endpoint
    #[serde(default)]
    pub resting_heart_rate: Option<i32>,
    #[serde(default)]
    pub max_heart_rate: Option<i32>,
    #[serde(default)]
    pub min_heart_rate: Option<i32>,
    // Activity fields
    pub total_steps: Option<i32>,
    #[serde(default)]
    pub highly_active_seconds: Option<i64>,
    #[serde(default)]
    pub active_seconds: Option<i64>,
    #[serde(default)]
    pub floors_ascended: Option<f64>,
    // Stress fields - may not be in this endpoint
    #[serde(default)]
    pub average_stress_level: Option<i32>,
    #[serde(default)]
    pub max_stress_level: Option<i32>,
    /// Seconds at rest (not stressed) during the day.
    #[serde(default)]
    pub rest_stress_duration: Option<i64>,
    /// Seconds at low stress during the day.
    #[serde(default)]
    pub low_stress_duration: Option<i64>,
    /// Seconds at medium stress during the day.
    #[serde(default)]
    pub medium_stress_duration: Option<i64>,
    /// Seconds at high stress during the day.
    #[serde(default)]
    pub high_stress_duration: Option<i64>,
    // Body battery fields - may not be in this endpoint
    #[serde(default)]
    pub body_battery_highest_value: Option<i32>,
    #[serde(default)]
    pub body_battery_lowest_value: Option<i32>,
    // Additional fields from actual API response
    #[serde(default)]
    pub total_kilocalories: Option<f64>,
    #[serde(default)]
    pub active_kilocalories: Option<f64>,
    #[serde(default)]
    pub bmr_kilocalories: Option<f64>,
    #[serde(default)]
    pub total_distance_meters: Option<f64>,
    /// Not sent by Garmin in any daily summary seen so far (issue #1). Pulse reads VO2 max from
    /// the maxmet endpoint and ignores this field. Kept so the 0.5 API does not change; remove it
    /// in the next breaking release.
    #[serde(default)]
    pub vo2_max_value: Option<f64>,
}

/// VO2 max: /metrics-service/metrics/maxmet/daily/{date}/{date}
/// One entry per day with a new estimate; `[]` (or no body) on other days. Declares only the
/// fields pulse reads; the stored body keeps the rest.
pub(crate) type GarminMaxMetResponse = Vec<GarminMaxMetEntry>;

/// One day of the maxmet response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GarminMaxMetEntry {
    /// The general (running and walking) estimate. `null` on days with only a cycling estimate.
    pub(crate) generic: Option<GarminVo2MaxEstimate>,
}

/// A VO2 max estimate, in ml/kg/min.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GarminVo2MaxEstimate {
    /// `YYYY-MM-DD` of the estimate. Required, so a changed shape fails to parse instead of
    /// reading as "no estimate".
    pub(crate) calendar_date: String,
    /// The estimate with one decimal, e.g. `47.6`.
    pub(crate) vo2_max_precise_value: Option<f64>,
    /// The estimate rounded to a whole number, as the Garmin app shows it.
    pub(crate) vo2_max_value: Option<f64>,
}

// ---------------------------------------------------------------------------
// API client
// ---------------------------------------------------------------------------

pub struct GarminApi<'a> {
    client: &'a reqwest::blocking::Client,
    access_token: String,
}

impl<'a> GarminApi<'a> {
    pub fn new(client: &'a reqwest::blocking::Client, access_token: String) -> Self {
        Self {
            client,
            access_token,
        }
    }

    /// Fetch and parse the sleep response for `date`.
    pub fn fetch_sleep(&self, date: NaiveDate) -> Result<GarminSleepResponse> {
        let endpoint = GarminEndpoint::Sleep;
        parse_body(endpoint, &self.fetch_body(endpoint, &date.to_string())?)
    }

    /// Fetch and parse the HRV response for `date`.
    pub fn fetch_hrv(&self, date: NaiveDate) -> Result<GarminHrvResponse> {
        let endpoint = GarminEndpoint::Hrv;
        parse_body(endpoint, &self.fetch_body(endpoint, &date.to_string())?)
    }

    /// Fetch and parse the Body Battery report for `date`.
    pub fn fetch_body_battery(&self, date: NaiveDate) -> Result<GarminBodyBatteryResponse> {
        let endpoint = GarminEndpoint::BodyBattery;
        parse_body(endpoint, &self.fetch_body(endpoint, &date.to_string())?)
    }

    /// Fetch and parse the daily summary for `date`.
    pub fn fetch_daily_summary(&self, date: NaiveDate) -> Result<GarminDailySummaryResponse> {
        let endpoint = GarminEndpoint::DailySummary;
        parse_body(endpoint, &self.fetch_body(endpoint, &date.to_string())?)
    }

    /// Fetch and parse the exercise sets of the Garmin activity `garmin_activity_id`.
    pub fn fetch_exercise_sets(
        &self,
        garmin_activity_id: &str,
    ) -> Result<GarminExerciseSetsResponse> {
        let endpoint = GarminEndpoint::ExerciseSets;
        parse_body(endpoint, &self.fetch_body(endpoint, garmin_activity_id)?)
    }

    /// Fetch the body of `endpoint` for `key` as received. Errors on a transport failure,
    /// a non-2xx status, or a body that cannot be read.
    pub(crate) fn fetch_body(&self, endpoint: GarminEndpoint, key: &str) -> Result<String> {
        let label = endpoint.label();
        let resp = self
            .client
            .get(endpoint.url(key))
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .with_context(|| format!("Failed to fetch {label}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("Garmin {label} API returned {status}: {body}");
        }

        resp.text()
            .with_context(|| format!("Failed to read {label} response"))
    }
}

/// Value of `raw_responses.provider` for bodies returned by Garmin Connect.
pub(crate) const PROVIDER: &str = "garmin";

/// A Garmin Connect endpoint pulse fetches. Its `name` is stored in `raw_responses.endpoint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GarminEndpoint {
    Sleep,
    Hrv,
    BodyBattery,
    DailySummary,
    ExerciseSets,
    MaxMet,
}

impl GarminEndpoint {
    /// Stored in `raw_responses.endpoint`. Never change a value once released:
    /// stored rows and migrations refer to it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Sleep => "sleep",
            Self::Hrv => "hrv",
            Self::BodyBattery => "body_battery",
            Self::DailySummary => "daily_summary",
            Self::ExerciseSets => "exercise_sets",
            Self::MaxMet => "maxmet",
        }
    }

    /// Used in error messages.
    fn label(self) -> &'static str {
        match self {
            Self::Sleep => "sleep",
            Self::Hrv => "HRV",
            Self::BodyBattery => "body battery",
            Self::DailySummary => "daily summary",
            Self::ExerciseSets => "exercise sets",
            Self::MaxMet => "VO2 max",
        }
    }

    /// `key` is a `YYYY-MM-DD` date, or the Garmin activity id for `ExerciseSets`.
    fn url(self, key: &str) -> String {
        match self {
            Self::Sleep => {
                format!("{CONNECT_API}/sleep-service/sleep/dailySleepData?date={key}")
            }
            Self::Hrv => format!("{CONNECT_API}/hrv-service/hrv/{key}"),
            Self::BodyBattery => format!(
                "{CONNECT_API}/wellness-service/wellness/bodyBattery/reports/daily?startDate={key}&endDate={key}"
            ),
            Self::DailySummary => {
                format!("{CONNECT_API}/usersummary-service/usersummary/daily/?calendarDate={key}")
            }
            Self::ExerciseSets => {
                format!("{CONNECT_API}/activity-service/activity/{key}/exerciseSets")
            }
            Self::MaxMet => {
                format!("{CONNECT_API}/metrics-service/metrics/maxmet/daily/{key}/{key}")
            }
        }
    }
}

/// Deserialize a Garmin body, fresh or stored. Errors with "Failed to parse {label} response".
pub(crate) fn parse_body<T: serde::de::DeserializeOwned>(
    endpoint: GarminEndpoint,
    body: &str,
) -> Result<T> {
    serde_json::from_str(body)
        .with_context(|| format!("Failed to parse {} response", endpoint.label()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_names_are_stable() {
        assert_eq!(GarminEndpoint::Sleep.name(), "sleep");
        assert_eq!(GarminEndpoint::Hrv.name(), "hrv");
        assert_eq!(GarminEndpoint::BodyBattery.name(), "body_battery");
        assert_eq!(GarminEndpoint::DailySummary.name(), "daily_summary");
        assert_eq!(GarminEndpoint::ExerciseSets.name(), "exercise_sets");
        assert_eq!(GarminEndpoint::MaxMet.name(), "maxmet");
        assert_eq!(PROVIDER, "garmin");
    }

    #[test]
    fn endpoint_urls_are_unchanged() {
        assert_eq!(
            GarminEndpoint::Sleep.url("2026-03-01"),
            "https://connectapi.garmin.com/sleep-service/sleep/dailySleepData?date=2026-03-01"
        );
        assert_eq!(
            GarminEndpoint::Hrv.url("2026-03-01"),
            "https://connectapi.garmin.com/hrv-service/hrv/2026-03-01"
        );
        assert_eq!(
            GarminEndpoint::BodyBattery.url("2026-03-01"),
            "https://connectapi.garmin.com/wellness-service/wellness/bodyBattery/reports/daily?startDate=2026-03-01&endDate=2026-03-01"
        );
        assert_eq!(
            GarminEndpoint::DailySummary.url("2026-03-01"),
            "https://connectapi.garmin.com/usersummary-service/usersummary/daily/?calendarDate=2026-03-01"
        );
        assert_eq!(
            GarminEndpoint::ExerciseSets.url("22005844470"),
            "https://connectapi.garmin.com/activity-service/activity/22005844470/exerciseSets"
        );
        assert_eq!(
            GarminEndpoint::MaxMet.url("2026-03-01"),
            "https://connectapi.garmin.com/metrics-service/metrics/maxmet/daily/2026-03-01/2026-03-01"
        );
    }

    #[test]
    fn parse_body_names_the_endpoint_in_the_error() {
        let err = parse_body::<GarminHrvResponse>(GarminEndpoint::Hrv, "nope").unwrap_err();
        assert!(format!("{err:#}").contains("Failed to parse HRV response"));
    }

    #[test]
    fn deserialize_sleep_response() {
        let json = r#"{
            "dailySleepDTO": {
                "sleepTimeInSeconds": 28800,
                "deepSleepSeconds": 7200,
                "remSleepSeconds": 5400,
                "lightSleepSeconds": 14400,
                "awakeSleepSeconds": 1800,
                "sleepStartTimestampLocal": 1772320200999,
                "sleepEndTimestampLocal": 1772348100000,
                "averageSpO2Value": 96.0,
                "averageRespirationValue": 14.5,
                "sleepScores": {
                    "overall": {
                        "value": 82,
                        "qualifierKey": "GOOD"
                    }
                }
            }
        }"#;
        let resp: GarminSleepResponse = serde_json::from_str(json).unwrap();
        let dto = resp.daily_sleep_dto.unwrap();
        assert_eq!(dto.sleep_time_in_seconds, Some(28800));
        assert_eq!(dto.deep_sleep_seconds, Some(7200));
        assert_eq!(dto.rem_sleep_in_seconds, Some(5400));
        assert_eq!(dto.sleep_start_timestamp_local, Some(1772320200999));
        assert_eq!(dto.sleep_end_timestamp_local, Some(1772348100000));
        assert_eq!(dto.average_sp_o2_value, Some(96.0));
        assert_eq!(dto.average_respiration_value, Some(14.5));
        assert_eq!(dto.sleep_scores.unwrap().overall.unwrap().value, Some(82));
    }

    #[test]
    fn deserialize_sleep_response_empty() {
        let json = r#"{"dailySleepDTO": null}"#;
        let resp: GarminSleepResponse = serde_json::from_str(json).unwrap();
        assert!(resp.daily_sleep_dto.is_none());
    }

    #[test]
    fn deserialize_sleep_response_no_dto_key() {
        let json = r#"{}"#;
        let resp: GarminSleepResponse = serde_json::from_str(json).unwrap();
        assert!(resp.daily_sleep_dto.is_none());
    }

    #[test]
    fn deserialize_hrv_response() {
        let json = r#"{
            "hrvSummary": {
                "lastNightAvg": 42.5,
                "lastNight5MinHigh": 68.0,
                "status": "BALANCED"
            }
        }"#;
        let resp: GarminHrvResponse = serde_json::from_str(json).unwrap();
        let summary = resp.hrv_summary.unwrap();
        assert_eq!(summary.last_night_avg, Some(42.5));
    }

    #[test]
    fn deserialize_hrv_response_empty() {
        let json = r#"{"hrvSummary": null}"#;
        let resp: GarminHrvResponse = serde_json::from_str(json).unwrap();
        assert!(resp.hrv_summary.is_none());
    }

    #[test]
    fn deserialize_daily_summary() {
        let json = r#"{
            "restingHeartRate": 55,
            "maxHeartRate": 142,
            "minHeartRate": 48,
            "totalSteps": 12345,
            "highlyActiveSeconds": 1800,
            "activeSeconds": 3600,
            "floorsAscended": 10,
            "averageStressLevel": 35,
            "maxStressLevel": 78,
            "bodyBatteryHighestValue": 95,
            "bodyBatteryLowestValue": 25,
            "restStressDuration": 28800,
            "lowStressDuration": 18000,
            "mediumStressDuration": 4200,
            "highStressDuration": 600,
            "totalKilocalories": 2450.0,
            "activeKilocalories": 612.0,
            "bmrKilocalories": 1838.0,
            "totalDistanceMeters": 8420
        }"#;
        let resp: GarminDailySummaryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.rest_stress_duration, Some(28800));
        assert_eq!(resp.low_stress_duration, Some(18000));
        assert_eq!(resp.medium_stress_duration, Some(4200));
        assert_eq!(resp.high_stress_duration, Some(600));
        assert_eq!(resp.total_kilocalories, Some(2450.0));
        assert_eq!(resp.active_kilocalories, Some(612.0));
        assert_eq!(resp.bmr_kilocalories, Some(1838.0));
        assert_eq!(resp.total_distance_meters, Some(8420.0));
        assert_eq!(resp.resting_heart_rate, Some(55));
        assert_eq!(resp.max_heart_rate, Some(142));
        assert_eq!(resp.total_steps, Some(12345));
        assert_eq!(resp.active_seconds, Some(3600));
        assert_eq!(resp.average_stress_level, Some(35));
        assert_eq!(resp.body_battery_highest_value, Some(95));
        assert_eq!(resp.body_battery_lowest_value, Some(25));
    }

    #[test]
    fn deserialize_body_battery_response() {
        let json = r#"[{"charged": 85, "drained": 62}]"#;
        let resp: GarminBodyBatteryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.len(), 1);
        assert_eq!(resp[0].charged, Some(85));
        assert_eq!(resp[0].drained, Some(62));
    }

    #[test]
    fn deserialize_body_battery_response_empty() {
        let json = r#"[]"#;
        let resp: GarminBodyBatteryResponse = serde_json::from_str(json).unwrap();
        assert!(resp.is_empty());
    }

    #[test]
    fn deserialize_daily_summary_sparse() {
        let json = r#"{"restingHeartRate": 60}"#;
        let resp: GarminDailySummaryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.resting_heart_rate, Some(60));
        assert!(resp.total_steps.is_none());
        assert!(resp.average_stress_level.is_none());
        assert!(resp.rest_stress_duration.is_none());
        assert!(resp.high_stress_duration.is_none());
    }

    #[test]
    fn deserialize_daily_summary_with_vo2_max() {
        let json = r#"{"restingHeartRate": 58, "vo2MaxValue": 48.5}"#;
        let resp: GarminDailySummaryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.resting_heart_rate, Some(58));
        assert!((resp.vo2_max_value.unwrap() - 48.5).abs() < 0.01);
    }

    #[test]
    fn deserialize_max_met_response() {
        let json = r#"[{"userId":12345678,"generic":{"calendarDate":"2026-03-01","vo2MaxPreciseValue":47.6,"vo2MaxValue":48.0,"fitnessAge":null,"fitnessAgeDescription":null,"maxMetCategory":0},"cycling":null,"heatAltitudeAcclimation":{"acclimationPercentage":0,"altitudeAcclimation":0,"heatAcclimationPercentage":0}}]"#;
        let resp: Option<GarminMaxMetResponse> = serde_json::from_str(json).unwrap();
        let entries = resp.unwrap();
        assert_eq!(entries.len(), 1);
        let generic = entries[0].generic.as_ref().unwrap();
        assert_eq!(generic.calendar_date, "2026-03-01");
        assert_eq!(generic.vo2_max_precise_value, Some(47.6));
        assert_eq!(generic.vo2_max_value, Some(48.0));
    }

    #[test]
    fn deserialize_max_met_tolerates_null_and_cycling_only_days() {
        let null: Option<GarminMaxMetResponse> = serde_json::from_str("null").unwrap();
        assert!(null.is_none());

        let empty: Option<GarminMaxMetResponse> = serde_json::from_str("[]").unwrap();
        assert!(empty.unwrap().is_empty());

        let cycling_only = r#"[{"userId":1,"generic":null,"cycling":{"calendarDate":"2026-03-01","vo2MaxPreciseValue":55.2,"vo2MaxValue":55,"maxMetCategory":1}}]"#;
        let entries: GarminMaxMetResponse = serde_json::from_str(cycling_only).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].generic.is_none());

        let integer = r#"[{"generic":{"calendarDate":"2026-03-01","vo2MaxValue":61}}]"#;
        let entries: GarminMaxMetResponse = serde_json::from_str(integer).unwrap();
        let generic = entries[0].generic.as_ref().unwrap();
        assert_eq!(generic.vo2_max_value, Some(61.0));
        assert_eq!(generic.vo2_max_precise_value, None);
    }
}
