use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;

use crate::config::IntervalsConfig;
use crate::db::queries;
use crate::db::Database;
use crate::models::{ExerciseSet, Workout};
use crate::providers::garmin::{api::GarminApi, auth, tokens};
use crate::providers::{DateRange, MetricType, Provider, SyncReport};

const BASE_URL: &str = "https://intervals.icu/api/v1";

// ---------------------------------------------------------------------------
// API response shapes (private — only used for deserialization)
// ---------------------------------------------------------------------------

/// Represents a single activity from the Intervals.icu activities list endpoint.
/// GET /api/v1/athlete/{id}/activities?oldest=...&newest=...
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ApiActivity {
    id: String,
    name: Option<String>,
    start_date_local: String,
    #[serde(default)]
    moving_time: Option<i64>,
    #[serde(rename = "type")]
    activity_type: Option<String>,
    icu_training_load: Option<f64>,
    calories: Option<f64>,
    average_heartrate: Option<f64>,
    max_heartrate: Option<f64>,
    /// Garmin Connect activity ID — used to fetch exercise sets.
    external_id: Option<String>,
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
                Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
                exercise_category: exercise
                    .and_then(|e| e.category.as_deref())
                    .map(format_garmin_name),
                exercise_name: exercise
                    .and_then(|e| e.name.as_deref())
                    .map(format_garmin_name)
                    .unwrap_or_else(|| "Unknown".into()),
                repetitions: s.repetition_count,
                weight_kg: s.weight.map(|w| w / 1000.0),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Provider implementation
// ---------------------------------------------------------------------------

pub struct IntervalsProvider {
    config: IntervalsConfig,
    client: reqwest::blocking::Client,
}

impl IntervalsProvider {
    pub fn new(config: IntervalsConfig) -> Self {
        Self {
            config,
            client: reqwest::blocking::Client::new(),
        }
    }

    /// Fetch the activities list for a date range.
    fn fetch_activities(&self, range: &DateRange) -> Result<Vec<ApiActivity>> {
        let athlete_id = if self.config.athlete_id.is_empty() {
            "0"
        } else {
            &self.config.athlete_id
        };

        let url = format!(
            "{BASE_URL}/athlete/{athlete_id}/activities?oldest={oldest}&newest={newest}",
            oldest = range.start,
            newest = range.end,
        );

        let resp = self
            .client
            .get(&url)
            .basic_auth("API_KEY", Some(&self.config.api_key))
            .send()
            .context("Failed to call Intervals.icu activities endpoint")?
            .error_for_status()
            .context("Intervals.icu activities endpoint returned an error")?;

        let activities: Vec<ApiActivity> = resp
            .json()
            .context("Failed to deserialize Intervals.icu activities response")?;

        Ok(activities)
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
            source: "intervals".into(),
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

        // 1. Fetch activities list for date range
        let activities = self.fetch_activities(range)?;

        for api_activity in &activities {
            // 2. Map and upsert the workout
            let workout = Self::map_workout(api_activity);
            if let Err(e) = queries::upsert_workout(db, &workout) {
                report
                    .errors
                    .push(format!("Failed to upsert workout {}: {e}", api_activity.id));
                continue;
            }
            report.records_synced += 1;

            // 3. For strength activities, fetch exercise sets from Garmin Connect
            let activity_type = api_activity.activity_type.as_deref().unwrap_or("");
            if Self::is_strength_activity(activity_type) {
                if let Some(garmin_id) = &api_activity.external_id {
                    if tokens::tokens_exist() {
                        match auth::ensure_valid_token(&self.client) {
                            Ok(token) => {
                                let garmin_api = GarminApi::new(&self.client, token);
                                match garmin_api.fetch_exercise_sets(garmin_id) {
                                    Ok(resp) => {
                                        if let Err(e) = queries::delete_exercise_sets_for_workout(
                                            db,
                                            &api_activity.id,
                                        ) {
                                            report.errors.push(format!(
                                                "Failed to clear exercise sets for {}: {e}",
                                                api_activity.id
                                            ));
                                            continue;
                                        }
                                        let sets = map_garmin_exercise_sets(
                                            &api_activity.id,
                                            &resp.exercise_sets,
                                        );
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
                                eprintln!(
                                    "[intervals] Garmin auth failed, skipping exercise sets: {e:#}"
                                );
                            }
                        }
                    }
                }
            }
        }

        // 4. Update sync state
        let now = Utc::now().to_rfc3339();
        queries::set_sync_state(db, "intervals_last_sync", &now)?;

        Ok(report)
    }

    fn supported_metrics(&self) -> &[MetricType] {
        &[MetricType::Workout]
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
    fn supported_metrics_contains_workout() {
        let provider = IntervalsProvider::new(IntervalsConfig {
            enabled: true,
            api_key: "test-key".into(),
            athlete_id: "0".into(),
        });
        assert!(matches!(
            provider.supported_metrics(),
            [MetricType::Workout]
        ));
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
            external_id: Some("22005844470".into()),
        };
        let w = IntervalsProvider::map_workout(&api);
        assert_eq!(w.id, "i12345");
        assert_eq!(w.name, Some("Morning Strength".into()));
        assert_eq!(w.duration_seconds, 3600);
        assert_eq!(w.calories, Some(350));
        assert_eq!(w.avg_hr, Some(120));
        assert_eq!(w.source, "intervals");
    }

    #[test]
    fn format_garmin_name_converts_screaming_snake_case() {
        assert_eq!(format_garmin_name("CABLE_EXTERNAL_ROTATION"), "Cable External Rotation");
        assert_eq!(format_garmin_name("SHOULDER_STABILITY"), "Shoulder Stability");
        assert_eq!(format_garmin_name("BENCH_PRESS"), "Bench Press");
        assert_eq!(format_garmin_name("SQUAT"), "Squat");
        // Leading underscore (Garmin sometimes prefixes names with _)
        assert_eq!(format_garmin_name("_90_DEGREE_CABLE_EXTERNAL_ROTATION"), "90 Degree Cable External Rotation");
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
        assert_eq!(sets[0].exercise_name, "Bench Press");
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
        assert_eq!(sets[0].exercise_name, "Squat");
        assert_eq!(sets[0].set_order, 1);
        assert_eq!(sets[1].exercise_name, "Deadlift");
        assert_eq!(sets[1].set_order, 2);
        assert_eq!(sets[1].weight_kg, Some(120.0));
    }
}
