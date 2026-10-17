use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::config::IntervalsConfig;
use crate::db::Database;
use crate::db::queries;
use crate::models::{ExerciseSet, Wellness, Workout};
use crate::providers::garmin::api::{
    self, GarminApi, GarminEndpoint, GarminExerciseSetsResponse, parse_body,
};
use crate::providers::garmin::{auth, tokens};
use crate::providers::{DateRange, MetricType, Provider, SyncReport, record_raw_response};

const BASE_URL: &str = "https://intervals.icu/api/v1";

// ---------------------------------------------------------------------------
// API response shapes (private — only used for deserialization)
// ---------------------------------------------------------------------------

/// Represents a single activity from the Intervals.icu activities list endpoint.
/// GET /api/v1/athlete/{id}/activities?oldest=...&newest=...
#[derive(Debug, Deserialize)]
struct ApiActivity {
    id: String,
    name: Option<String>,
    start_date_local: String,
    #[serde(default)]
    moving_time: Option<i64>,
    #[serde(rename = "type")]
    activity_type: Option<String>,
    /// Intervals.icu's training load. The API documents an integer; read as `f64` so a fraction
    /// cannot fail the whole activity. `training_load` decides what counts.
    icu_training_load: Option<f64>,
    calories: Option<f64>,
    average_heartrate: Option<f64>,
    max_heartrate: Option<f64>,
    /// Distance in meters. Null, missing or 0 for an activity without distance.
    distance: Option<f64>,
    /// Average speed in meters per second, as the device recorded it. Not the body's `pace`
    /// field, which despite its name is also m/s (distance / moving_time).
    average_speed: Option<f64>,
    /// Seconds in each heart rate zone, lowest first, against the zones configured in
    /// Intervals.icu. Kept as raw JSON so an unexpected shape costs only the zone times, not the
    /// whole activity; `hr_zone_seconds` decides what counts.
    icu_hr_zone_times: Option<serde_json::Value>,
    /// Garmin Connect activity ID — used to fetch exercise sets.
    external_id: Option<String>,
}

/// One record from the Intervals.icu wellness list, one per day.
/// GET /api/v1/athlete/{id}/wellness?oldest=...&newest=...
/// Only the fields pulse stores; serde ignores the rest, and the stored body keeps them.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiWellness {
    /// The day, `YYYY-MM-DD`.
    id: String,
    /// Fitness (chronic training load).
    ctl: Option<f64>,
    /// Fatigue (acute training load).
    atl: Option<f64>,
    /// Body weight in kg.
    weight: Option<f64>,
    /// Body fat in percent.
    body_fat: Option<f64>,
    /// `true` when Intervals.icu copied `weight` from an earlier record for an activity on this
    /// day, rather than a weigh-in. `measured_weight` drops such weights.
    temp_weight: Option<bool>,
}

// ---------------------------------------------------------------------------
// Garmin exercise set mapping
// ---------------------------------------------------------------------------

/// Convert a Garmin SCREAMING_SNAKE_CASE name to Title Case.
/// e.g. "CABLE_EXTERNAL_ROTATION" → "Cable External Rotation"
fn format_garmin_name(s: &str) -> String {
    s.split('_')
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What `format_garmin_name` makes of Garmin's `UNKNOWN` placeholder.
const UNKNOWN_LABEL: &str = "Unknown";

/// Format a Garmin category or exercise name, or `None` when Garmin gave no real value.
///
/// A missing value, an empty value, and Garmin's `UNKNOWN` placeholder all mean "not
/// identified". The checks are made after formatting, so a value made only of underscores,
/// which formats to an empty string, counts as empty too.
fn garmin_label(raw: Option<&str>) -> Option<String> {
    raw.map(format_garmin_name)
        .filter(|label| !label.is_empty() && label != UNKNOWN_LABEL)
}

/// Map Garmin Connect exercise sets to our ExerciseSet model, filtering REST sets.
fn map_garmin_exercise_sets(
    workout_id: &str,
    api_sets: &[crate::providers::garmin::api::GarminExerciseSet],
) -> Vec<ExerciseSet> {
    api_sets
        .iter()
        .filter(|s| s.set_type.as_deref().map(str::to_uppercase).as_deref() != Some("REST"))
        .filter(|s| !s.exercises.is_empty())
        .enumerate()
        .map(|(i, s)| {
            let exercise = s.exercises.first();
            ExerciseSet {
                id: None,
                workout_id: workout_id.to_string(),
                set_order: (i + 1) as i32,
                exercise_category: garmin_label(exercise.and_then(|e| e.category.as_deref())),
                exercise_name: garmin_label(exercise.and_then(|e| e.name.as_deref())),
                repetitions: s.repetition_count,
                weight_kg: s.weight.map(|w| w / 1000.0),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Provider implementation
// ---------------------------------------------------------------------------

/// Value of `raw_responses.provider` for Intervals.icu bodies.
const PROVIDER: &str = "intervals";

/// `raw_responses.endpoint` for one activity object from the activities list.
/// Never change it once released: stored rows and migrations refer to it.
const ACTIVITY_ENDPOINT: &str = "activity";

/// `raw_responses.endpoint` for one record of the wellness list, keyed by its `id` (the day,
/// `YYYY-MM-DD`). Never change it once released: stored rows and migrations refer to it.
const WELLNESS_ENDPOINT: &str = "wellness";

pub struct IntervalsProvider {
    config: IntervalsConfig,
    client: reqwest::blocking::Client,
    raw_responses: bool,
}

/// One object from an Intervals.icu list body, kept as received, with its `id`.
#[derive(Debug, PartialEq)]
struct ListItem {
    id: String,
    body: String,
}

/// Just the `id` of a list object, so the rest of the object can be kept as received.
#[derive(Deserialize)]
struct ItemId {
    id: String,
}

/// Split a list body into its objects, each kept as received. `list` names the list in errors
/// ("activities", "wellness"). Errors when the body is not a JSON array or an object has no
/// string `id`.
fn split_list(list_body: &str, list: &str) -> Result<Vec<ListItem>> {
    let items: Vec<Box<RawValue>> = serde_json::from_str(list_body)
        .with_context(|| format!("Intervals.icu {list} list is not a JSON array"))?;
    items
        .into_iter()
        .map(|item| {
            let ItemId { id } = serde_json::from_str(item.get()).with_context(|| {
                format!("An item of the Intervals.icu {list} list has no string id")
            })?;
            Ok(ListItem {
                id,
                body: item.get().to_string(),
            })
        })
        .collect()
}

/// Split an activities-list body into its activity objects, each kept as received.
fn split_activities(list_body: &str) -> Result<Vec<ListItem>> {
    split_list(list_body, "activities")
}

/// Split a wellness-list body into its daily records, each kept as received.
fn split_wellness(list_body: &str) -> Result<Vec<ListItem>> {
    split_list(list_body, "wellness")
}

/// Parse one activity body, fresh or stored.
fn parse_activity(body: &str) -> Result<ApiActivity> {
    serde_json::from_str(body).context("Failed to parse Intervals.icu activity")
}

/// Parse one wellness record body, fresh or stored.
fn parse_wellness(body: &str) -> Result<ApiWellness> {
    serde_json::from_str(body).context("Failed to parse Intervals.icu wellness record")
}

/// The day of a wellness record: `Some` only for a canonical `YYYY-MM-DD` date.
fn wellness_date(id: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(id, "%Y-%m-%d")
        .ok()
        .filter(|d| d.to_string() == id)
}

/// Weight from a weigh-in: kept when above 0 and not copied from an earlier day (`tempWeight`).
fn measured_weight(weight: Option<f64>, temp_weight: Option<bool>) -> Option<f64> {
    weight.filter(|w| *w > 0.0 && temp_weight != Some(true))
}

/// Body fat kept only strictly between 0 and 100 percent.
fn body_fat_percent(body_fat: Option<f64>) -> Option<f64> {
    body_fat.filter(|b| *b > 0.0 && *b < 100.0)
}

/// Map a parsed wellness record for `date` to our row. `ctl` and `atl` are copied as received.
fn map_wellness(date: NaiveDate, api: &ApiWellness) -> Wellness {
    Wellness {
        date: date.to_string(),
        weight_kg: measured_weight(api.weight, api.temp_weight),
        body_fat_percent: body_fat_percent(api.body_fat),
        ctl: api.ctl,
        atl: api.atl,
        source: "intervals".into(),
    }
}

/// Map a wellness record body, fresh or stored, to our row. Errors when the body does not parse
/// or its `id` is not a `YYYY-MM-DD` date.
fn wellness_from_body(body: &str) -> Result<Wellness> {
    let api = parse_wellness(body)?;
    let date = wellness_date(&api.id)
        .with_context(|| format!("Wellness id {:?} is not a YYYY-MM-DD date", api.id))?;
    Ok(map_wellness(date, &api))
}

/// Zone times from the body's `icu_hr_zone_times`: `Some` only for an array of JSON integers
/// that are all >= 0 with at least one above 0; missing, null, empty, all-zero or any other
/// shape is `None` (no zone time recorded). The v9 migration repeats this rule in SQL;
/// `migration_v9_backfill_matches_the_rust_mapping` pins the two together.
fn hr_zone_seconds(times: Option<&serde_json::Value>) -> Option<Vec<i64>> {
    let seconds = times?
        .as_array()?
        .iter()
        .map(|zone| zone.as_i64().filter(|s| *s >= 0))
        .collect::<Option<Vec<i64>>>()?;
    seconds.iter().any(|s| *s > 0).then_some(seconds)
}

/// Training load from the body's `icu_training_load` in whole points: truncated toward zero and
/// kept only from 1 to `i32::MAX`. Missing, null, below 1 (0 means no load computed) or beyond
/// `i32::MAX` is `None`. The v10 migration repeats this rule in SQL;
/// `migration_v10_backfill_matches_the_rust_mapping` pins the two together.
fn training_load(load: Option<f64>) -> Option<i32> {
    load.map(f64::trunc)
        .filter(|l| (1.0..=f64::from(i32::MAX)).contains(l))
        .map(|l| l as i32)
}

/// Map a Garmin exercise-sets body, fresh or stored, to our sets for `workout_id`.
fn sets_from_body(workout_id: &str, body: &str) -> Result<Vec<ExerciseSet>> {
    let resp: GarminExerciseSetsResponse = parse_body(GarminEndpoint::ExerciseSets, body)?;
    Ok(map_garmin_exercise_sets(workout_id, &resp.exercise_sets))
}

/// Refill workouts, exercise sets and wellness from the bodies in `raw_responses`. No network.
///
/// An activity whose exercise-sets body is not stored keeps the sets it has. A body that does
/// not parse is reported in `errors`; a database error is returned.
pub(crate) fn rebuild(db: &Database) -> Result<SyncReport> {
    let mut report = SyncReport {
        provider: "intervals".to_string(),
        records_synced: 0,
        errors: Vec::new(),
    };

    for key in queries::raw_response_keys(db, PROVIDER, ACTIVITY_ENDPOINT)? {
        let Some(body) = queries::get_raw_response(db, PROVIDER, ACTIVITY_ENDPOINT, &key)? else {
            continue;
        };
        let activity = match parse_activity(&body) {
            Ok(activity) => activity,
            Err(e) => {
                report.errors.push(format!("activity {key}: {e:#}"));
                continue;
            }
        };

        queries::upsert_workout(db, &IntervalsProvider::map_workout(&activity))?;
        report.records_synced += 1;

        let activity_type = activity.activity_type.as_deref().unwrap_or("");
        if IntervalsProvider::is_strength_activity(activity_type)
            && let Some(garmin_id) = &activity.external_id
            && let Some(sets_body) = queries::get_raw_response(
                db,
                api::PROVIDER,
                GarminEndpoint::ExerciseSets.name(),
                garmin_id,
            )?
        {
            match sets_from_body(&activity.id, &sets_body) {
                Ok(sets) => {
                    queries::delete_exercise_sets_for_workout(db, &activity.id)?;
                    for set in &sets {
                        queries::insert_exercise_set(db, set)?;
                    }
                    report.records_synced += sets.len();
                }
                Err(e) => report
                    .errors
                    .push(format!("exercise sets for {key}: {e:#}")),
            }
        }
    }

    for key in queries::raw_response_keys(db, PROVIDER, WELLNESS_ENDPOINT)? {
        let Some(body) = queries::get_raw_response(db, PROVIDER, WELLNESS_ENDPOINT, &key)? else {
            continue;
        };
        match wellness_from_body(&body) {
            Ok(wellness) => {
                queries::upsert_wellness(db, &wellness)?;
                report.records_synced += 1;
            }
            Err(e) => report.errors.push(format!("wellness {key}: {e:#}")),
        }
    }

    Ok(report)
}

impl IntervalsProvider {
    pub fn new(config: IntervalsConfig) -> Self {
        Self {
            config,
            client: reqwest::blocking::Client::new(),
            raw_responses: true,
        }
    }

    /// Keep (`true`, the default) or stop keeping each response body in `raw_responses`.
    pub fn with_raw_responses(mut self, enabled: bool) -> Self {
        self.raw_responses = enabled;
        self
    }

    /// URL of an athlete list endpoint (`list` = "activities" or "wellness") for `range`.
    /// An empty `athlete_id` becomes "0", the athlete who owns the API key.
    fn list_url(&self, list: &str, range: &DateRange) -> String {
        let athlete_id = if self.config.athlete_id.is_empty() {
            "0"
        } else {
            &self.config.athlete_id
        };

        format!(
            "{BASE_URL}/athlete/{athlete_id}/{list}?oldest={oldest}&newest={newest}",
            oldest = range.start,
            newest = range.end,
        )
    }

    /// GET `list_url` with basic auth; the body text of a 2xx response.
    fn fetch_list(&self, list: &str, range: &DateRange) -> Result<String> {
        let resp = self
            .client
            .get(self.list_url(list, range))
            .basic_auth("API_KEY", Some(&self.config.api_key))
            .send()
            .with_context(|| format!("Failed to call Intervals.icu {list} endpoint"))?
            .error_for_status()
            .with_context(|| format!("Intervals.icu {list} endpoint returned an error"))?;

        resp.text()
            .with_context(|| format!("Failed to read Intervals.icu {list} response"))
    }

    /// Fetch the activities list for a date range, one body per activity.
    fn fetch_activity_bodies(&self, range: &DateRange) -> Result<Vec<ListItem>> {
        let text = self.fetch_list("activities", range)?;
        split_activities(&text).context("Failed to deserialize Intervals.icu activities response")
    }

    /// Fetch the wellness list for a date range, one body per day.
    fn fetch_wellness_bodies(&self, range: &DateRange) -> Result<Vec<ListItem>> {
        let text = self.fetch_list("wellness", range)?;
        split_wellness(&text).context("Failed to deserialize Intervals.icu wellness response")
    }

    /// Returns true if the activity type looks like a strength/weight training workout.
    fn is_strength_activity(activity_type: &str) -> bool {
        let lower = activity_type.to_lowercase();
        lower.contains("weight") || lower.contains("strength")
    }

    /// Convert an API activity into our Workout model.
    fn map_workout(api: &ApiActivity) -> Workout {
        Workout {
            id: api.id.clone(),
            name: api.name.clone(),
            start_time: api.start_date_local.clone(),
            duration_seconds: api.moving_time.unwrap_or(0),
            activity_type: api.activity_type.clone().unwrap_or_default(),
            calories: api.calories.map(|c| c as i32),
            avg_hr: api.average_heartrate.map(|h| h as i32),
            max_hr: api.max_heartrate.map(|h| h as i32),
            distance_meters: api.distance.filter(|d| *d > 0.0),
            avg_speed_mps: api.average_speed.filter(|s| *s > 0.0),
            hr_zone_seconds: hr_zone_seconds(api.icu_hr_zone_times.as_ref()),
            training_load: training_load(api.icu_training_load),
            source: "intervals".into(),
        }
    }

    /// Store, map and upsert each activity.
    fn sync_activities(&self, db: &Database, activities: &[ListItem], report: &mut SyncReport) {
        for item in activities {
            // Store the body before mapping it, so a mapped row is never newer than its body.
            if let Err(e) = record_raw_response(
                db,
                self.raw_responses,
                PROVIDER,
                ACTIVITY_ENDPOINT,
                &item.id,
                &item.body,
            ) {
                report
                    .errors
                    .push(format!("Failed to store response for {}: {e}", item.id));
                continue;
            }
            let api_activity = match parse_activity(&item.body) {
                Ok(activity) => activity,
                Err(e) => {
                    report
                        .errors
                        .push(format!("Failed to parse activity {}: {e:#}", item.id));
                    continue;
                }
            };

            // 2. Map and upsert the workout
            let workout = Self::map_workout(&api_activity);
            if let Err(e) = queries::upsert_workout(db, &workout) {
                report
                    .errors
                    .push(format!("Failed to upsert workout {}: {e}", api_activity.id));
                continue;
            }
            report.records_synced += 1;

            // 3. For strength activities, fetch exercise sets from Garmin Connect
            let activity_type = api_activity.activity_type.as_deref().unwrap_or("");
            if Self::is_strength_activity(activity_type)
                && let Some(garmin_id) = &api_activity.external_id
                && tokens::tokens_exist()
            {
                match auth::ensure_valid_token(&self.client) {
                    Ok(token) => {
                        let garmin_api = GarminApi::new(&self.client, token);
                        match garmin_api.fetch_body(GarminEndpoint::ExerciseSets, garmin_id) {
                            Ok(body) => {
                                if let Err(e) = record_raw_response(
                                    db,
                                    self.raw_responses,
                                    api::PROVIDER,
                                    GarminEndpoint::ExerciseSets.name(),
                                    garmin_id,
                                    &body,
                                ) {
                                    report.errors.push(format!(
                                        "Failed to store exercise sets response for {}: {e}",
                                        api_activity.id
                                    ));
                                    continue;
                                }
                                let sets = match sets_from_body(&api_activity.id, &body) {
                                    Ok(sets) => sets,
                                    Err(e) => {
                                        report.errors.push(format!(
                                            "Failed to parse exercise sets for {}: {e:#}",
                                            api_activity.id
                                        ));
                                        continue;
                                    }
                                };
                                if let Err(e) =
                                    queries::delete_exercise_sets_for_workout(db, &api_activity.id)
                                {
                                    report.errors.push(format!(
                                        "Failed to clear exercise sets for {}: {e}",
                                        api_activity.id
                                    ));
                                    continue;
                                }
                                for set in &sets {
                                    if let Err(e) = queries::insert_exercise_set(db, set) {
                                        report.errors.push(format!(
                                            "Failed to insert exercise set for {}: {e}",
                                            api_activity.id
                                        ));
                                    }
                                }
                                report.records_synced += sets.len();
                            }
                            Err(e) => {
                                report.errors.push(format!(
                                    "Failed to fetch exercise sets for {} from Garmin: {e}",
                                    api_activity.id
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[intervals] Garmin auth failed, skipping exercise sets: {e:#}");
                    }
                }
            }
        }
    }

    /// Store each record's body, then map and upsert it. A failure costs only that record.
    fn sync_wellness(&self, db: &Database, records: &[ListItem], report: &mut SyncReport) {
        for item in records {
            // Store the body before mapping it, so a mapped row is never newer than its body.
            if let Err(e) = record_raw_response(
                db,
                self.raw_responses,
                PROVIDER,
                WELLNESS_ENDPOINT,
                &item.id,
                &item.body,
            ) {
                report.errors.push(format!(
                    "Failed to store wellness response for {}: {e}",
                    item.id
                ));
                continue;
            }
            let wellness = match wellness_from_body(&item.body) {
                Ok(wellness) => wellness,
                Err(e) => {
                    report
                        .errors
                        .push(format!("Failed to map wellness record {}: {e:#}", item.id));
                    continue;
                }
            };
            if let Err(e) = queries::upsert_wellness(db, &wellness) {
                report
                    .errors
                    .push(format!("Failed to upsert wellness for {}: {e}", item.id));
                continue;
            }
            report.records_synced += 1;
        }
    }
}

impl Provider for IntervalsProvider {
    fn name(&self) -> &str {
        "intervals"
    }

    fn sync(&self, db: &Database, range: &DateRange) -> Result<SyncReport> {
        let mut report = SyncReport {
            provider: self.name().to_string(),
            records_synced: 0,
            errors: Vec::new(),
        };

        // Each list is fetched on its own, so one failing costs only that list.
        let activities_fetched = match self.fetch_activity_bodies(range) {
            Ok(activities) => {
                self.sync_activities(db, &activities, &mut report);
                true
            }
            Err(e) => {
                report.errors.push(format!("{e:#}"));
                false
            }
        };
        let wellness_fetched = match self.fetch_wellness_bodies(range) {
            Ok(records) => {
                self.sync_wellness(db, &records, &mut report);
                true
            }
            Err(e) => {
                report.errors.push(format!("{e:#}"));
                false
            }
        };

        // Update sync state, only when both lists were fetched.
        if activities_fetched && wellness_fetched {
            let now = Utc::now().to_rfc3339();
            queries::set_sync_state(db, "intervals_last_sync", &now)?;
        }

        Ok(report)
    }

    fn supported_metrics(&self) -> &[MetricType] {
        &[MetricType::Workout, MetricType::Wellness]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::IntervalsConfig;

    #[test]
    fn provider_name() {
        let provider = IntervalsProvider::new(IntervalsConfig {
            enabled: true,
            api_key: "test-key".into(),
            athlete_id: "0".into(),
        });
        assert_eq!(provider.name(), "intervals");
    }

    #[test]
    fn supported_metrics_are_workouts_and_wellness() {
        let provider = IntervalsProvider::new(IntervalsConfig {
            enabled: true,
            api_key: "test-key".into(),
            athlete_id: "0".into(),
        });
        assert!(matches!(
            provider.supported_metrics(),
            [MetricType::Workout, MetricType::Wellness]
        ));
    }

    #[test]
    fn endpoint_names_are_pinned() {
        assert_eq!(WELLNESS_ENDPOINT, "wellness");
        assert_eq!(ACTIVITY_ENDPOINT, "activity");
    }

    #[test]
    fn list_url_names_the_list_and_the_range() {
        let range = DateRange {
            start: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(),
        };
        let provider = |athlete_id: &str| {
            IntervalsProvider::new(IntervalsConfig {
                enabled: true,
                api_key: "test-key".into(),
                athlete_id: athlete_id.into(),
            })
        };
        assert_eq!(
            provider("").list_url("wellness", &range),
            "https://intervals.icu/api/v1/athlete/0/wellness?oldest=2026-10-01&newest=2026-10-07"
        );
        assert_eq!(
            provider("i123").list_url("activities", &range),
            "https://intervals.icu/api/v1/athlete/i123/activities?oldest=2026-10-01&newest=2026-10-07"
        );
    }

    #[test]
    fn is_strength_activity_detection() {
        assert!(IntervalsProvider::is_strength_activity("WeightTraining"));
        assert!(IntervalsProvider::is_strength_activity("Strength"));
        assert!(IntervalsProvider::is_strength_activity("strength_training"));
        assert!(!IntervalsProvider::is_strength_activity("Run"));
        assert!(!IntervalsProvider::is_strength_activity("Ride"));
    }

    #[test]
    fn map_workout_from_api() {
        let api = ApiActivity {
            id: "i12345".into(),
            name: Some("Morning Strength".into()),
            start_date_local: "2024-01-15T08:30:00".into(),
            moving_time: Some(3600),
            activity_type: Some("WeightTraining".into()),
            icu_training_load: Some(45.2),
            calories: Some(350.0),
            average_heartrate: Some(120.0),
            max_heartrate: Some(155.0),
            distance: None,
            average_speed: None,
            icu_hr_zone_times: None,
            external_id: Some("22005844470".into()),
        };
        let w = IntervalsProvider::map_workout(&api);
        assert_eq!(w.id, "i12345");
        assert_eq!(w.name, Some("Morning Strength".into()));
        assert_eq!(w.duration_seconds, 3600);
        assert_eq!(w.calories, Some(350));
        assert_eq!(w.avg_hr, Some(120));
        assert_eq!(w.distance_meters, None);
        assert_eq!(w.avg_speed_mps, None);
        assert_eq!(w.hr_zone_seconds, None);
        assert_eq!(w.training_load, Some(45), "45.2 is truncated");
        assert_eq!(w.source, "intervals");
    }

    /// An activity as the API sends it, with only the distance and speed under test filled in.
    fn walk_with(distance: Option<f64>, average_speed: Option<f64>) -> ApiActivity {
        ApiActivity {
            id: "i1".into(),
            name: None,
            start_date_local: "2026-10-15T07:00:00".into(),
            moving_time: Some(3540),
            activity_type: Some("Walk".into()),
            icu_training_load: None,
            calories: None,
            average_heartrate: None,
            max_heartrate: None,
            distance,
            average_speed,
            icu_hr_zone_times: None,
            external_id: None,
        }
    }

    #[test]
    fn map_workout_keeps_distance_and_speed() {
        let w = IntervalsProvider::map_workout(&walk_with(Some(5043.21), Some(1.417)));
        assert_eq!(w.distance_meters, Some(5043.21));
        assert_eq!(w.avg_speed_mps, Some(1.417));
    }

    #[test]
    fn map_workout_treats_zero_or_negative_as_absent() {
        for value in [0.0, -1.0] {
            let w = IntervalsProvider::map_workout(&walk_with(Some(value), Some(value)));
            assert_eq!(w.distance_meters, None, "distance {value}");
            assert_eq!(w.avg_speed_mps, None, "speed {value}");
        }
    }

    #[test]
    fn parse_activity_reads_distance_and_average_speed() {
        let read = |body: &str| {
            let api = parse_activity(body).unwrap();
            (api.distance, api.average_speed)
        };
        assert_eq!(
            read(
                r#"{"id":"i1","start_date_local":"2026-10-15T07:00:00","distance":5043.21,"average_speed":1.417,"pace":1.4375}"#
            ),
            (Some(5043.21), Some(1.417))
        );
        assert_eq!(
            read(
                r#"{"id":"i2","start_date_local":"2026-10-15T07:00:00","distance":1670,"average_speed":null}"#
            ),
            (Some(1670.0), None)
        );
        assert_eq!(
            read(r#"{"id":"i3","start_date_local":"2026-10-15T07:00:00"}"#),
            (None, None)
        );
    }

    #[test]
    fn format_garmin_name_converts_screaming_snake_case() {
        assert_eq!(
            format_garmin_name("CABLE_EXTERNAL_ROTATION"),
            "Cable External Rotation"
        );
        assert_eq!(
            format_garmin_name("SHOULDER_STABILITY"),
            "Shoulder Stability"
        );
        assert_eq!(format_garmin_name("BENCH_PRESS"), "Bench Press");
        assert_eq!(format_garmin_name("SQUAT"), "Squat");
        // Leading underscore (Garmin sometimes prefixes names with _)
        assert_eq!(
            format_garmin_name("_90_DEGREE_CABLE_EXTERNAL_ROTATION"),
            "90 Degree Cable External Rotation"
        );
    }

    #[test]
    fn map_garmin_exercise_sets_maps_correctly() {
        use crate::providers::garmin::api::{GarminExercise, GarminExerciseSet};

        let api_sets = vec![
            GarminExerciseSet {
                exercises: vec![GarminExercise {
                    category: Some("CHEST".into()),
                    name: Some("BENCH_PRESS".into()),
                }],
                repetition_count: Some(10),
                weight: Some(60000.0), // 60 kg in grams
                set_type: Some("ACTIVE".into()),
            },
            GarminExerciseSet {
                exercises: vec![GarminExercise {
                    category: Some("CHEST".into()),
                    name: Some("BENCH_PRESS".into()),
                }],
                repetition_count: Some(8),
                weight: Some(65000.0), // 65 kg in grams
                set_type: Some("ACTIVE".into()),
            },
        ];
        let sets = map_garmin_exercise_sets("w-001", &api_sets);
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].set_order, 1);
        assert_eq!(sets[0].exercise_name, Some("Bench Press".into()));
        assert_eq!(sets[0].exercise_category, Some("Chest".into()));
        assert_eq!(sets[1].set_order, 2);
        assert_eq!(sets[1].weight_kg, Some(65.0));
        assert_eq!(sets[0].workout_id, "w-001");
    }

    #[test]
    fn map_garmin_exercise_sets_filters_rest_sets() {
        use crate::providers::garmin::api::{GarminExercise, GarminExerciseSet};

        let api_sets = vec![
            GarminExerciseSet {
                exercises: vec![GarminExercise {
                    category: Some("LEGS".into()),
                    name: Some("SQUAT".into()),
                }],
                repetition_count: Some(5),
                weight: Some(100000.0),
                set_type: Some("ACTIVE".into()),
            },
            GarminExerciseSet {
                exercises: vec![],
                repetition_count: None,
                weight: None,
                set_type: Some("REST".into()),
            },
            GarminExerciseSet {
                exercises: vec![GarminExercise {
                    category: Some("BACK".into()),
                    name: Some("DEADLIFT".into()),
                }],
                repetition_count: Some(3),
                weight: Some(120000.0),
                set_type: Some("ACTIVE".into()),
            },
        ];
        let sets = map_garmin_exercise_sets("w-002", &api_sets);
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].exercise_name, Some("Squat".into()));
        assert_eq!(sets[0].set_order, 1);
        assert_eq!(sets[1].exercise_name, Some("Deadlift".into()));
        assert_eq!(sets[1].set_order, 2);
        assert_eq!(sets[1].weight_kg, Some(120.0));
    }

    #[test]
    fn garmin_label_is_none_without_a_real_value() {
        assert_eq!(garmin_label(None), None);
        assert_eq!(garmin_label(Some("UNKNOWN")), None);
        assert_eq!(garmin_label(Some("unknown")), None);
        // Empty, or made only of separators, formats to an empty string.
        assert_eq!(garmin_label(Some("")), None);
        assert_eq!(garmin_label(Some("_")), None);
        assert_eq!(garmin_label(Some("__")), None);
        assert_eq!(
            garmin_label(Some("BENCH_PRESS")),
            Some("Bench Press".into())
        );
        // Only the bare placeholder counts, not a name that starts with it.
        assert_eq!(
            garmin_label(Some("UNKNOWN_EXERCISE")),
            Some("Unknown Exercise".into())
        );
    }

    #[test]
    fn map_garmin_exercise_sets_leaves_missing_labels_none() {
        use crate::providers::garmin::api::{GarminExercise, GarminExerciseSet};

        let set = |category: Option<&str>, name: Option<&str>| GarminExerciseSet {
            exercises: vec![GarminExercise {
                category: category.map(Into::into),
                name: name.map(Into::into),
            }],
            repetition_count: Some(10),
            weight: None,
            set_type: Some("ACTIVE".into()),
        };
        let api_sets = vec![
            set(Some("CURL"), None),
            set(Some("UNKNOWN"), None),
            set(None, None),
            set(None, Some("PULL_UP")),
        ];
        let sets = map_garmin_exercise_sets("w-003", &api_sets);
        let labels: Vec<(Option<&str>, Option<&str>)> = sets
            .iter()
            .map(|s| (s.exercise_category.as_deref(), s.exercise_name.as_deref()))
            .collect();
        assert_eq!(
            labels,
            [
                (Some("Curl"), None),
                (None, None),
                (None, None),
                (None, Some("Pull Up")),
            ]
        );
        // A set without labels is still a set.
        assert_eq!(sets[1].set_order, 2);
        assert_eq!(sets[1].repetitions, Some(10));
    }

    const ACTIVITIES_BODY: &str = r#"[
  {"id":"i101","name":"Push","start_date_local":"2026-03-01T08:00:00","moving_time":3600,"type":"WeightTraining","calories":350,"average_heartrate":120,"max_heartrate":155,"icu_training_load":52,"external_id":"22005844470","icu_hr_zone_times":[1500,1200,600,300,0],"icu_ftp":1.50},
  {"id": "i102", "start_date_local": "2026-03-02T07:00:00", "type": "Run", "icu_hr_zone_times": [0, 0, 0], "average_speed": 2.9, "distance": 5012.5}
]"#;
    const SETS_BODY: &str = r#"{"exerciseSets":[{"exercises":[{"category":"BENCH_PRESS","name":"BARBELL_BENCH_PRESS"}],"repetitionCount":10,"weight":60000.0,"setType":"ACTIVE"},{"exercises":[],"setType":"REST"}]}"#;

    /// The exact text of one activity object inside `ACTIVITIES_BODY`.
    fn slice_of_list(start_marker: &str, end_marker: &str) -> &'static str {
        let start = ACTIVITIES_BODY.find(start_marker).unwrap();
        let end = ACTIVITIES_BODY.find(end_marker).unwrap() + end_marker.len();
        &ACTIVITIES_BODY[start..end]
    }

    fn store_activities(db: &Database) {
        for item in split_activities(ACTIVITIES_BODY).unwrap() {
            queries::upsert_raw_response(db, PROVIDER, ACTIVITY_ENDPOINT, &item.id, &item.body)
                .unwrap();
        }
    }

    #[test]
    fn split_activities_keeps_each_object_as_received() {
        let items = split_activities(ACTIVITIES_BODY).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "i101");
        assert_eq!(
            items[0].body,
            slice_of_list(r#"{"id":"i101""#, r#""icu_ftp":1.50}"#)
        );
        assert!(items[0].body.contains("1.50"));
        assert_eq!(items[1].id, "i102");
        assert_eq!(
            items[1].body,
            slice_of_list(r#"{"id": "i102""#, r#""distance": 5012.5}"#)
        );
    }

    #[test]
    fn split_activities_rejects_a_body_that_is_not_an_array() {
        assert!(split_activities(r#"{"id":"i1"}"#).is_err());
    }

    #[test]
    fn split_activities_rejects_an_object_without_an_id() {
        assert!(split_activities(r#"[{"name":"x"}]"#).is_err());
    }

    /// Four wellness records. The second is spaced and carries a copied weight, the third is all
    /// zeros, and the fourth has an id that is not a date.
    const WELLNESS_BODY: &str = r#"[
  {"id":"2026-10-14","ctl":52.34,"atl":61.0,"rampRate":3.1,"ctlLoad":0.0,"atlLoad":0.0,"weight":78.4,"bodyFat":18.2,"restingHR":52,"tempWeight":false,"tempRestingHR":false,"sportInfo":[{"type":"Run","eftp":null,"wPrime":null,"pMax":null}]},
  {"id": "2026-10-15", "ctl": 53.0, "atl": 66.5, "weight": 78.4, "bodyFat": null, "tempWeight": true},
  {"id":"2026-10-16","ctl":0,"atl":0,"weight":0,"bodyFat":0},
  {"id":"2026/10/17","ctl":50.0,"atl":50.0}
]"#;

    fn wellness_row(
        date: &str,
        weight_kg: Option<f64>,
        body_fat_percent: Option<f64>,
        ctl: Option<f64>,
        atl: Option<f64>,
    ) -> Wellness {
        Wellness {
            date: date.into(),
            weight_kg,
            body_fat_percent,
            ctl,
            atl,
            source: "intervals".into(),
        }
    }

    /// The rows `WELLNESS_BODY` maps to, oldest first. The fourth record does not map.
    fn mapped_wellness_rows() -> [Wellness; 3] {
        [
            wellness_row(
                "2026-10-14",
                Some(78.4),
                Some(18.2),
                Some(52.34),
                Some(61.0),
            ),
            wellness_row("2026-10-15", None, None, Some(53.0), Some(66.5)),
            wellness_row("2026-10-16", None, None, Some(0.0), Some(0.0)),
        ]
    }

    fn october_14_to_16() -> DateRange {
        DateRange {
            start: NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 10, 16).unwrap(),
        }
    }

    fn provider_with_raw_responses(enabled: bool) -> IntervalsProvider {
        IntervalsProvider::new(IntervalsConfig {
            enabled: true,
            api_key: "test-key".into(),
            athlete_id: "0".into(),
        })
        .with_raw_responses(enabled)
    }

    fn empty_report() -> SyncReport {
        SyncReport {
            provider: "intervals".into(),
            records_synced: 0,
            errors: Vec::new(),
        }
    }

    #[test]
    fn split_wellness_keeps_each_record_as_received() {
        let items = split_wellness(WELLNESS_BODY).unwrap();
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            ["2026-10-14", "2026-10-15", "2026-10-16", "2026/10/17"]
        );
        assert_eq!(
            items[1].body,
            r#"{"id": "2026-10-15", "ctl": 53.0, "atl": 66.5, "weight": 78.4, "bodyFat": null, "tempWeight": true}"#
        );
        assert!(items[0].body.contains(r#""sportInfo""#));
    }

    #[test]
    fn split_wellness_rejects_a_body_that_is_not_an_array_or_a_record_without_an_id() {
        assert!(split_wellness(r#"{"id":"2026-10-14"}"#).is_err());
        assert!(split_wellness(r#"[{"ctl":1.0}]"#).is_err());
    }

    #[test]
    fn wellness_date_accepts_only_canonical_dates() {
        assert!(wellness_date("2026-10-15").is_some());
        for id in [
            "2026-1-5",
            "2026-02-30",
            "20261015",
            "2026/10/15",
            "2026-10-15T00:00:00",
            "",
        ] {
            assert!(wellness_date(id).is_none(), "{id:?}");
        }
    }

    #[test]
    fn measured_weight_drops_copies_and_non_positive_values() {
        assert_eq!(measured_weight(Some(78.4), None), Some(78.4));
        assert_eq!(measured_weight(Some(78.4), Some(false)), Some(78.4));
        assert_eq!(measured_weight(Some(78.4), Some(true)), None);
        assert_eq!(measured_weight(Some(0.0), None), None);
        assert_eq!(measured_weight(Some(-1.0), None), None);
        assert_eq!(measured_weight(None, Some(false)), None);
    }

    #[test]
    fn body_fat_percent_keeps_values_strictly_between_zero_and_one_hundred() {
        for kept in [0.1, 18.2, 99.9] {
            assert_eq!(body_fat_percent(Some(kept)), Some(kept));
        }
        for dropped in [None, Some(0.0), Some(-3.0), Some(100.0), Some(100.5)] {
            assert_eq!(body_fat_percent(dropped), None, "{dropped:?}");
        }
    }

    #[test]
    fn wellness_from_body_maps_each_record() {
        let items = split_wellness(WELLNESS_BODY).unwrap();
        for (item, expected) in items.iter().zip(mapped_wellness_rows()) {
            assert_eq!(wellness_from_body(&item.body).unwrap(), expected);
        }
        assert_eq!(
            wellness_from_body(r#"{"id":"2026-10-18"}"#).unwrap(),
            wellness_row("2026-10-18", None, None, None, None)
        );
        assert!(wellness_from_body(&items[3].body).is_err());
        assert!(wellness_from_body(r#"{"weight":78.4}"#).is_err());
        assert!(wellness_from_body(r#"{"id":"2026-10-18","weight":"78.4"}"#).is_err());
    }

    #[test]
    fn sync_wellness_stores_each_body_then_maps_it() {
        let db = Database::open_memory().unwrap();
        let items = split_wellness(WELLNESS_BODY).unwrap();
        let mut report = empty_report();

        provider_with_raw_responses(true).sync_wellness(&db, &items, &mut report);

        assert_eq!(report.records_synced, 3);
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(
            report.errors[0].contains("2026/10/17"),
            "{:?}",
            report.errors
        );
        for item in &items {
            assert!(
                queries::get_raw_response(&db, PROVIDER, WELLNESS_ENDPOINT, &item.id)
                    .unwrap()
                    .is_some(),
                "{} is stored before it is mapped",
                item.id
            );
        }
        let mut expected = mapped_wellness_rows().to_vec();
        expected.reverse();
        assert_eq!(
            queries::query_wellness(&db, &october_14_to_16()).unwrap(),
            expected
        );
    }

    #[test]
    fn sync_wellness_without_raw_storage_keeps_no_body() {
        let db = Database::open_memory().unwrap();
        queries::upsert_raw_response(
            &db,
            PROVIDER,
            WELLNESS_ENDPOINT,
            "2026-10-14",
            r#"{"id":"2026-10-14","weight":70.0}"#,
        )
        .unwrap();
        let items = split_wellness(WELLNESS_BODY).unwrap();
        let mut report = empty_report();

        provider_with_raw_responses(false).sync_wellness(&db, &items, &mut report);

        for item in &items {
            assert!(
                queries::get_raw_response(&db, PROVIDER, WELLNESS_ENDPOINT, &item.id)
                    .unwrap()
                    .is_none(),
                "{}",
                item.id
            );
        }
        assert_eq!(report.records_synced, 3);
        assert_eq!(
            queries::query_wellness(&db, &october_14_to_16())
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn rebuild_refills_wellness() {
        let db = Database::open_memory().unwrap();
        for item in split_wellness(WELLNESS_BODY).unwrap() {
            queries::upsert_raw_response(&db, PROVIDER, WELLNESS_ENDPOINT, &item.id, &item.body)
                .unwrap();
        }
        queries::upsert_wellness(
            &db,
            &wellness_row("2026-10-14", Some(99.0), None, None, None),
        )
        .unwrap();

        let report = rebuild(&db).unwrap();

        assert_eq!(report.records_synced, 3);
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(
            report.errors[0].starts_with("wellness 2026/10/17"),
            "{:?}",
            report.errors
        );
        let mut expected = mapped_wellness_rows().to_vec();
        expected.reverse();
        assert_eq!(
            queries::query_wellness(&db, &october_14_to_16()).unwrap(),
            expected,
            "the stale weight was replaced"
        );
    }

    #[test]
    fn sets_from_body_maps_and_drops_rest_sets() {
        let sets = sets_from_body("i101", SETS_BODY).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(
            sets[0].exercise_name.as_deref(),
            Some("Barbell Bench Press")
        );
        assert_eq!(sets[0].weight_kg, Some(60.0));
    }

    #[test]
    fn with_raw_responses_sets_the_switch() {
        let config = IntervalsConfig {
            enabled: true,
            api_key: "test-key".into(),
            athlete_id: "0".into(),
        };
        assert!(IntervalsProvider::new(config.clone()).raw_responses);
        assert!(
            !IntervalsProvider::new(config)
                .with_raw_responses(false)
                .raw_responses
        );
    }

    #[test]
    fn rebuild_refills_workouts_and_sets() {
        let db = Database::open_memory().unwrap();
        store_activities(&db);
        queries::upsert_raw_response(
            &db,
            api::PROVIDER,
            GarminEndpoint::ExerciseSets.name(),
            "22005844470",
            SETS_BODY,
        )
        .unwrap();

        let report = rebuild(&db).unwrap();

        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.records_synced, 3);
        let calories: Option<i32> = db
            .conn()
            .query_row("SELECT calories FROM workouts WHERE id = 'i101'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(calories, Some(350));
        let activity_type: String = db
            .conn()
            .query_row(
                "SELECT activity_type FROM workouts WHERE id = 'i102'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(activity_type, "Run");
        let extras = |id: &str| -> (Option<f64>, Option<f64>) {
            db.conn()
                .query_row(
                    "SELECT distance_meters, avg_speed_mps FROM workouts WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
        };
        assert_eq!(extras("i102"), (Some(5012.5), Some(2.9)));
        assert_eq!(extras("i101"), (None, None));
        let zones = |id: &str| -> Option<String> {
            db.conn()
                .query_row(
                    "SELECT hr_zone_seconds FROM workouts WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(zones("i101").as_deref(), Some("[1500,1200,600,300,0]"));
        assert_eq!(zones("i102"), None, "all zeros are no zone time");
        let load = |id: &str| -> Option<i32> {
            db.conn()
                .query_row(
                    "SELECT training_load FROM workouts WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(load("i101"), Some(52));
        assert_eq!(load("i102"), None);
        assert_eq!(queries::query_exercise_sets(&db, "i101").unwrap().len(), 1);
    }

    /// The v10 migration repeats `training_load`'s rule in SQL. Both must store the same value
    /// for the same stored body, malformed ones included.
    #[test]
    fn migration_v10_backfill_matches_the_rust_mapping() {
        let bodies = [
            ("i501", r#""icu_training_load":43"#),
            ("i502", r#""icu_training_load":0"#),
            ("i503", r#""icu_training_load":57.9"#),
            ("i504", ""),
            ("i505", r#""icu_training_load":null"#),
            ("i506", r#""icu_training_load":-5"#),
            ("i507", r#""icu_training_load":0.9"#),
            ("i508", r#""icu_training_load":3000000000"#),
            ("i509", r#""icu_training_load":2147483647.5"#),
            ("i510", r#""icu_training_load":9223372036854775808"#),
            ("i511", r#""icu_training_load":1.0"#),
        ];
        let db = Database::open_memory().unwrap();
        let mut mapped = Vec::new();
        for (id, field) in bodies {
            let separator = if field.is_empty() { "" } else { "," };
            let body = format!(
                r#"{{"id":"{id}","start_date_local":"2026-10-15T07:00:00","type":"Walk"{separator}{field}}}"#
            );
            let w = IntervalsProvider::map_workout(&parse_activity(&body).unwrap());
            queries::upsert_raw_response(&db, PROVIDER, ACTIVITY_ENDPOINT, &w.id, &body).unwrap();
            // A stale value the migration must overwrite either way.
            let before = Workout {
                training_load: Some(999),
                ..w.clone()
            };
            queries::upsert_workout(&db, &before).unwrap();
            mapped.push(w);
        }

        crate::db::migrations::migrate_v10(&db).unwrap();

        for w in &mapped {
            let stored: Option<i64> = db
                .conn()
                .query_row(
                    "SELECT training_load FROM workouts WHERE id = ?1",
                    [&w.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(stored, w.training_load.map(i64::from), "{}", w.id);
        }
        // Not vacuous: only these bodies map to a value.
        for w in &mapped {
            let expected = match w.id.as_str() {
                "i501" => Some(43),
                "i503" => Some(57),
                "i509" => Some(i32::MAX),
                "i511" => Some(1),
                _ => None,
            };
            assert_eq!(w.training_load, expected, "{}", w.id);
        }
    }

    #[test]
    fn training_load_keeps_whole_points_from_one_to_i32_max() {
        assert_eq!(training_load(Some(43.0)), Some(43));
        assert_eq!(training_load(Some(57.9)), Some(57));
        assert_eq!(training_load(Some(1.0)), Some(1));
        assert_eq!(training_load(Some(2147483647.5)), Some(i32::MAX));
        for value in [
            None,
            Some(0.0),
            Some(0.9),
            Some(-5.0),
            Some(2147483648.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(training_load(value), None, "{value:?}");
        }
    }

    /// The v9 migration repeats `hr_zone_seconds`'s rule in SQL. Both must store the same text
    /// for the same stored body, malformed ones included.
    #[test]
    fn migration_v9_backfill_matches_the_rust_mapping() {
        let bodies = [
            r#"{"id":"i401","start_date_local":"2026-10-15T07:00:00","type":"Walk","moving_time":3540,"icu_hr_zone_times":[491,158,150,110,28,0,0]}"#,
            r#"{"id":"i402","start_date_local":"2026-10-14T18:00:00","type":"WeightTraining","icu_hr_zone_times":[0,0,0,0,0,0,0]}"#,
            r#"{"id":"i403","start_date_local":"2026-10-13T07:00:00","type":"Run","icu_hr_zone_times": [0, 0, 600, 1200, 30]}"#,
            r#"{"id":"i404","start_date_local":"2026-10-12T07:00:00","type":"Walk"}"#,
            r#"{"id":"i405","start_date_local":"2026-10-11T07:00:00","type":"Walk","icu_hr_zone_times":null}"#,
            r#"{"id":"i406","start_date_local":"2026-10-10T07:00:00","type":"Walk","icu_hr_zone_times":[]}"#,
            r#"{"id":"i407","start_date_local":"2026-10-09T07:00:00","type":"Walk","icu_hr_zone_times":[491.0,158,0]}"#,
            r#"{"id":"i408","start_date_local":"2026-10-08T07:00:00","type":"Walk","icu_hr_zone_times":[491,null,0]}"#,
            r#"{"id":"i409","start_date_local":"2026-10-07T07:00:00","type":"Walk","icu_hr_zone_times":[491,-1,0]}"#,
            r#"{"id":"i410","start_date_local":"2026-10-06T07:00:00","type":"Walk","icu_hr_zone_times":"491,158"}"#,
            r#"{"id":"i411","start_date_local":"2026-10-05T07:00:00","type":"Walk","icu_hr_zone_times":[9223372036854775808,1]}"#,
            r#"{"id":"i412","start_date_local":"2026-10-04T07:00:00","type":"Walk","icu_hr_zone_times":[true,158]}"#,
        ];
        let db = Database::open_memory().unwrap();
        let mut mapped = Vec::new();
        for body in bodies {
            let w = IntervalsProvider::map_workout(&parse_activity(body).unwrap());
            queries::upsert_raw_response(&db, PROVIDER, ACTIVITY_ENDPOINT, &w.id, body).unwrap();
            // A stale value the migration must overwrite either way.
            let before = Workout {
                hr_zone_seconds: Some(vec![9, 9, 9]),
                ..w.clone()
            };
            queries::upsert_workout(&db, &before).unwrap();
            mapped.push(w);
        }

        crate::db::migrations::migrate_v9(&db).unwrap();

        for w in &mapped {
            let stored: Option<String> = db
                .conn()
                .query_row(
                    "SELECT hr_zone_seconds FROM workouts WHERE id = ?1",
                    [&w.id],
                    |r| r.get(0),
                )
                .unwrap();
            let expected = w
                .hr_zone_seconds
                .as_ref()
                .map(|z| serde_json::to_string(z).unwrap());
            assert_eq!(stored, expected, "{}", w.id);
        }
        // Not vacuous: only the two well-formed bodies with time map to a value.
        for w in &mapped {
            let expected = match w.id.as_str() {
                "i401" => Some(vec![491, 158, 150, 110, 28, 0, 0]),
                "i403" => Some(vec![0, 0, 600, 1200, 30]),
                _ => None,
            };
            assert_eq!(w.hr_zone_seconds, expected, "{}", w.id);
        }
    }

    #[test]
    fn hr_zone_seconds_keeps_only_arrays_of_non_negative_integers_with_time() {
        use serde_json::json;
        assert_eq!(
            hr_zone_seconds(Some(&json!([491, 158, 150, 110, 28, 0, 0]))),
            Some(vec![491, 158, 150, 110, 28, 0, 0])
        );
        assert_eq!(
            hr_zone_seconds(Some(&json!([0, 0, 600, 1200, 30]))),
            Some(vec![0, 0, 600, 1200, 30])
        );
        for value in [
            json!(null),
            json!([]),
            json!([0, 0, 0, 0, 0, 0, 0]),
            json!([491.0, 158, 0]),
            json!([491, null, 0]),
            json!([491, -1, 0]),
            json!("491,158"),
            json!(["491", 158]),
            json!({"z1": 491}),
            json!([true, 158]),
            json!([[491], 158]),
        ] {
            assert_eq!(hr_zone_seconds(Some(&value)), None, "{value}");
        }
        assert_eq!(hr_zone_seconds(None), None);
    }

    #[test]
    fn parse_activity_keeps_the_activity_when_zone_times_are_malformed() {
        for times in [r#""oops""#, r#"{"z1":491}"#] {
            let body = format!(
                r#"{{"id":"i1","start_date_local":"2026-10-15T07:00:00","icu_hr_zone_times":{times}}}"#
            );
            let api = parse_activity(&body).unwrap();
            assert_eq!(
                IntervalsProvider::map_workout(&api).hr_zone_seconds,
                None,
                "{times}"
            );
        }
        for body in [
            r#"{"id":"i1","start_date_local":"2026-10-15T07:00:00"}"#,
            r#"{"id":"i1","start_date_local":"2026-10-15T07:00:00","icu_hr_zone_times":null}"#,
        ] {
            assert_eq!(parse_activity(body).unwrap().icu_hr_zone_times, None);
        }
    }

    /// The v8 migration repeats `map_workout`'s distance and speed rules in SQL. Both must give
    /// the same values for the same stored body.
    #[test]
    fn migration_v8_backfill_matches_the_rust_mapping() {
        let bodies = [
            r#"{"id":"i301","start_date_local":"2026-10-15T07:00:00","type":"Walk","moving_time":3540,"distance":5043.21,"average_speed":1.417,"pace":1.4375}"#,
            r#"{"id":"i302","start_date_local":"2026-10-14T18:00:00","type":"WeightTraining","moving_time":3600,"distance":0,"average_speed":0.0}"#,
            r#"{"id":"i303","start_date_local":"2026-10-13T07:00:00","type":"Walk","distance":1670,"average_speed":null}"#,
            r#"{"id":"i304","start_date_local":"2026-10-12T07:00:00","type":"Run"}"#,
            r#"{"id":"i305","start_date_local":"2026-10-11T07:00:00","type":"Run","distance":-5.0,"average_speed":-1.0}"#,
        ];
        let db = Database::open_memory().unwrap();
        let mut mapped = Vec::new();
        for body in bodies {
            let w = IntervalsProvider::map_workout(&parse_activity(body).unwrap());
            queries::upsert_raw_response(&db, PROVIDER, ACTIVITY_ENDPOINT, &w.id, body).unwrap();
            // The row as 0.8 left it: no distance, no speed.
            let before = Workout {
                distance_meters: None,
                avg_speed_mps: None,
                ..w.clone()
            };
            queries::upsert_workout(&db, &before).unwrap();
            mapped.push(w);
        }

        crate::db::migrations::migrate_v8(&db).unwrap();

        for w in &mapped {
            let stored: (Option<f64>, Option<f64>) = db
                .conn()
                .query_row(
                    "SELECT distance_meters, avg_speed_mps FROM workouts WHERE id = ?1",
                    [&w.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(stored, (w.distance_meters, w.avg_speed_mps), "{}", w.id);
        }
        // Not vacuous: the mapped values themselves are what the bodies say.
        let values = |i: usize| (mapped[i].distance_meters, mapped[i].avg_speed_mps);
        assert_eq!(values(0), (Some(5043.21), Some(1.417)));
        assert_eq!(values(1), (None, None));
        assert_eq!(values(2), (Some(1670.0), None));
    }

    #[test]
    fn rebuild_keeps_sets_when_their_body_is_not_stored() {
        let db = Database::open_memory().unwrap();
        let items = split_activities(ACTIVITIES_BODY).unwrap();
        let activity = parse_activity(&items[0].body).unwrap();
        queries::upsert_workout(&db, &IntervalsProvider::map_workout(&activity)).unwrap();
        queries::insert_exercise_set(
            &db,
            &ExerciseSet {
                id: None,
                workout_id: "i101".into(),
                set_order: 1,
                exercise_category: None,
                exercise_name: Some("Squat".into()),
                repetitions: Some(5),
                weight_kg: Some(100.0),
            },
        )
        .unwrap();
        queries::upsert_raw_response(&db, PROVIDER, ACTIVITY_ENDPOINT, "i101", &items[0].body)
            .unwrap();

        rebuild(&db).unwrap();

        assert_eq!(queries::query_exercise_sets(&db, "i101").unwrap().len(), 1);
    }
}
