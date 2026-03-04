//! Daily heart rate and cardiovascular metrics.

use serde::{Deserialize, Serialize};

/// Daily heart rate and cardiovascular metrics synced from a wearable device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heart {
    /// Date in `YYYY-MM-DD` format (primary key in the DB).
    pub date: String,
    /// Resting heart rate in beats per minute (bpm).
    pub resting_hr: Option<i32>,
    /// Maximum heart rate recorded during the day (bpm).
    pub max_hr: Option<i32>,
    /// Minimum heart rate recorded during the day (bpm).
    pub min_hr: Option<i32>,
    /// Average heart-rate variability in milliseconds (RMSSD or device equivalent).
    pub hrv_avg: Option<f64>,
    /// VO2 Max estimate in ml/kg/min reported by the device.
    pub vo2_max: Option<f64>,
    /// Provider that supplied this record (e.g. `"garmin"`).
    pub source: String,
}
