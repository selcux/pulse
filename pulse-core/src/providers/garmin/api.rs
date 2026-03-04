use anyhow::{bail, Context, Result};
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
    pub average_sp_o2_value: Option<f64>,
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
    // Fitness
    #[serde(default)]
    pub vo2_max_value: Option<f64>,
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

    pub fn fetch_sleep(&self, date: NaiveDate) -> Result<GarminSleepResponse> {
        let url = format!(
            "{CONNECT_API}/sleep-service/sleep/dailySleepData?date={date}"
        );
        self.get_json(&url, "sleep")
    }

    pub fn fetch_hrv(&self, date: NaiveDate) -> Result<GarminHrvResponse> {
        let url = format!("{CONNECT_API}/hrv-service/hrv/{date}");
        self.get_json(&url, "HRV")
    }

    pub fn fetch_body_battery(&self, date: NaiveDate) -> Result<GarminBodyBatteryResponse> {
        let url = format!(
            "{CONNECT_API}/wellness-service/wellness/bodyBattery/reports/daily?startDate={date}&endDate={date}"
        );
        self.get_json(&url, "body battery")
    }

    pub fn fetch_daily_summary(&self, date: NaiveDate) -> Result<GarminDailySummaryResponse> {
        let url = format!(
            "{CONNECT_API}/usersummary-service/usersummary/daily/?calendarDate={date}"
        );
        self.get_json(&url, "daily summary")
    }

    pub fn fetch_exercise_sets(&self, garmin_activity_id: &str) -> Result<GarminExerciseSetsResponse> {
        let url = format!(
            "{CONNECT_API}/activity-service/activity/{garmin_activity_id}/exerciseSets"
        );
        self.get_json(&url, "exercise sets")
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str, label: &str) -> Result<T> {
        let resp = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .with_context(|| format!("Failed to fetch {label}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("Garmin {label} API returned {status}: {body}");
        }

        let body_text = resp.text().unwrap_or_default();
        serde_json::from_str::<T>(&body_text)
            .with_context(|| format!("Failed to parse {label} response"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_sleep_response() {
        let json = r#"{
            "dailySleepDTO": {
                "sleepTimeInSeconds": 28800,
                "deepSleepSeconds": 7200,
                "remSleepSeconds": 5400,
                "lightSleepSeconds": 14400,
                "awakeSleepSeconds": 1800,
                "averageSpO2Value": 96.0,
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
        assert_eq!(
            dto.sleep_scores.unwrap().overall.unwrap().value,
            Some(82)
        );
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
            "bodyBatteryLowestValue": 25
        }"#;
        let resp: GarminDailySummaryResponse = serde_json::from_str(json).unwrap();
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
    }

    #[test]
    fn deserialize_daily_summary_with_vo2_max() {
        let json = r#"{"restingHeartRate": 58, "vo2MaxValue": 48.5}"#;
        let resp: GarminDailySummaryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.resting_heart_rate, Some(58));
        assert!((resp.vo2_max_value.unwrap() - 48.5).abs() < 0.01);
    }
}
