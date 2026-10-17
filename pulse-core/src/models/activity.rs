//! Daily activity summary (steps, active minutes, floors, calories, distance).

use serde::{Deserialize, Serialize};

/// Daily activity summary synced from a wearable device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    /// Date in `YYYY-MM-DD` format (primary key in the DB).
    pub date: String,
    /// Total steps taken during the day.
    pub steps: Option<i32>,
    /// Minutes of moderate-to-vigorous activity.
    pub active_minutes: Option<i32>,
    /// Floors climbed (from barometric altimeter).
    pub floors: Option<i32>,
    /// Total calories burned during the day (active plus resting), in kcal.
    pub total_kcal: Option<f64>,
    /// Calories burned through activity, in kcal.
    pub active_kcal: Option<f64>,
    /// Resting (basal metabolic rate) calories, in kcal.
    pub bmr_kcal: Option<f64>,
    /// Distance covered during the day, in meters.
    pub distance_meters: Option<f64>,
    /// Provider that supplied this record (e.g. `"garmin"`).
    pub source: String,
}
