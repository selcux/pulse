//! Individual exercise set within a strength workout.

use serde::{Deserialize, Serialize};

/// A single set of a strength exercise within a [`Workout`](crate::models::Workout).
///
/// Exercise sets are linked to their parent workout via [`workout_id`](Self::workout_id)
/// and ordered by [`set_order`](Self::set_order).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExerciseSet {
    /// Auto-assigned database row ID (`None` before insertion).
    pub id: Option<i64>,
    /// Foreign key referencing the parent [`Workout::id`](crate::models::Workout::id).
    pub workout_id: String,
    /// 1-based position of this set within the workout.
    pub set_order: i32,
    /// Broad exercise category (e.g. `"chest"`, `"back"`, `"legs"`).
    pub exercise_category: Option<String>,
    /// Exercise name (e.g. `"bench press"`, `"squat"`).
    pub exercise_name: String,
    /// Number of repetitions performed.
    pub repetitions: Option<i32>,
    /// Load used in kilograms (`None` for bodyweight exercises).
    pub weight_kg: Option<f64>,
}
