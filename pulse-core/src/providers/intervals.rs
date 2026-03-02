use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;

use crate::config::IntervalsConfig;
use crate::db::queries;
use crate::db::Database;
use crate::models::{ExerciseSet, Workout};
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
    start_date_local: String,
    #[serde(default)]
    moving_time: Option<i64>,
    #[serde(rename = "type")]
    activity_type: Option<String>,
    icu_training_load: Option<f64>,
    calories: Option<f64>,
    average_heartrate: Option<f64>,
    max_heartrate: Option<f64>,
}

/// Represents the full activity detail (used to extract exercise sets).
/// GET /api/v1/activity/{id}
#[derive(Debug, Deserialize)]
struct ApiActivityDetail {
    #[serde(default)]
    icu_sets: Option<Vec<ApiExerciseSet>>,
}

/// A single exercise set from Intervals.icu's `icu_sets` array.
#[derive(Debug, Deserialize)]
struct ApiExerciseSet {
    exercise: Option<String>,
    reps: Option<i32>,
    weight: Option<f64>,
    category: Option<String>,
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

    /// Fetch full activity detail (needed for exercise sets on strength workouts).
    fn fetch_activity_detail(&self, activity_id: &str) -> Result<ApiActivityDetail> {
        let url = format!("{BASE_URL}/activity/{activity_id}");

        let resp = self
            .client
            .get(&url)
            .basic_auth("API_KEY", Some(&self.config.api_key))
            .send()
            .context("Failed to call Intervals.icu activity detail endpoint")?
            .error_for_status()
            .context("Intervals.icu activity detail endpoint returned an error")?;

        let detail: ApiActivityDetail = resp
            .json()
            .context("Failed to deserialize Intervals.icu activity detail")?;

        Ok(detail)
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
            start_time: api.start_date_local.clone(),
            duration_seconds: api.moving_time.unwrap_or(0),
            activity_type: api.activity_type.clone().unwrap_or_default(),
            calories: api.calories.map(|c| c as i32),
            avg_hr: api.average_heartrate.map(|h| h as i32),
            max_hr: api.max_heartrate.map(|h| h as i32),
            source: "intervals".into(),
        }
    }

    /// Convert API exercise sets into our ExerciseSet models.
    fn map_exercise_sets(workout_id: &str, api_sets: &[ApiExerciseSet]) -> Vec<ExerciseSet> {
        api_sets
            .iter()
            .enumerate()
            .map(|(i, s)| ExerciseSet {
                id: None,
                workout_id: workout_id.to_string(),
                set_order: (i + 1) as i32,
                exercise_category: s.category.clone(),
                exercise_name: s.exercise.clone().unwrap_or_else(|| "Unknown".into()),
                repetitions: s.reps,
                weight_kg: s.weight,
            })
            .collect()
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

            // 3. For strength activities, fetch detail and sync exercise sets
            let activity_type = api_activity.activity_type.as_deref().unwrap_or("");
            if Self::is_strength_activity(activity_type) {
                match self.fetch_activity_detail(&api_activity.id) {
                    Ok(detail) => {
                        if let Some(api_sets) = &detail.icu_sets {
                            // Delete existing sets, then insert fresh ones
                            if let Err(e) =
                                queries::delete_exercise_sets_for_workout(db, &api_activity.id)
                            {
                                report.errors.push(format!(
                                    "Failed to clear exercise sets for {}: {e}",
                                    api_activity.id
                                ));
                                continue;
                            }

                            let sets = Self::map_exercise_sets(&api_activity.id, api_sets);
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
                    }
                    Err(e) => {
                        report.errors.push(format!(
                            "Failed to fetch activity detail for {}: {e}",
                            api_activity.id
                        ));
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
            start_date_local: "2024-01-15T08:30:00".into(),
            moving_time: Some(3600),
            activity_type: Some("WeightTraining".into()),
            icu_training_load: Some(45.2),
            calories: Some(350.0),
            average_heartrate: Some(120.0),
            max_heartrate: Some(155.0),
        };
        let w = IntervalsProvider::map_workout(&api);
        assert_eq!(w.id, "i12345");
        assert_eq!(w.duration_seconds, 3600);
        assert_eq!(w.calories, Some(350));
        assert_eq!(w.avg_hr, Some(120));
        assert_eq!(w.source, "intervals");
    }

    #[test]
    fn map_exercise_sets_from_api() {
        let api_sets = vec![
            ApiExerciseSet {
                exercise: Some("Bench Press".into()),
                reps: Some(10),
                weight: Some(60.0),
                category: Some("Chest".into()),
            },
            ApiExerciseSet {
                exercise: Some("Bench Press".into()),
                reps: Some(8),
                weight: Some(65.0),
                category: Some("Chest".into()),
            },
        ];
        let sets = IntervalsProvider::map_exercise_sets("w-001", &api_sets);
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].set_order, 1);
        assert_eq!(sets[0].exercise_name, "Bench Press");
        assert_eq!(sets[1].set_order, 2);
        assert_eq!(sets[1].weight_kg, Some(65.0));
        assert_eq!(sets[0].workout_id, "w-001");
    }
}
