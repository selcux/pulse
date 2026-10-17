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
    /// Distance covered, in meters. `None` when the provider recorded no distance (strength
    /// training, for example).
    pub distance_meters: Option<f64>,
    /// Average speed over the workout as the recording device reported it, in meters per second.
    /// `None` when not recorded. Pace per kilometer is `1000 / avg_speed_mps` seconds.
    pub avg_speed_mps: Option<f64>,
    /// Seconds spent in each heart rate zone, lowest zone first (`[0]` is Z1), as Intervals.icu
    /// counted them against the zones configured there for this workout. The length is the
    /// athlete's zone count. The times add up to the time with a heart rate reading, which can
    /// differ from `duration_seconds` by a minute or two. `None` when the provider recorded no zone
    /// time: no heart rate, or every zone at 0.
    pub hr_zone_seconds: Option<Vec<i64>>,
    /// Provider that supplied this record (e.g. `"intervals"`, `"garmin"`).
    pub source: String,
}
