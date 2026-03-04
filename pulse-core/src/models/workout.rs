//! Individual workout session data from Garmin Connect or Intervals.icu.

use serde::{Deserialize, Serialize};

/// A single workout session synced from a fitness platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workout {
    /// Unique workout identifier from the source platform (primary key in the DB).
    pub id: String,
    /// Optional workout name or title assigned by the user or device.
    pub name: Option<String>,
    /// Start time in ISO 8601 format (`YYYY-MM-DDTHH:MM:SS`).
    pub start_time: String,
    /// Total workout duration in seconds.
    pub duration_seconds: i64,
    /// Activity type string (e.g. `"running"`, `"strength"`, `"cycling"`).
    pub activity_type: String,
    /// Estimated calories burned during the workout.
    pub calories: Option<i32>,
    /// Average heart rate during the workout (bpm).
    pub avg_hr: Option<i32>,
    /// Peak heart rate during the workout (bpm).
    pub max_hr: Option<i32>,
    /// Provider that supplied this record (e.g. `"intervals"`, `"garmin"`).
    pub source: String,
}
