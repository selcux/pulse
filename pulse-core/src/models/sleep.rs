//! Nightly sleep summary including stage breakdown and quality score.

use serde::{Deserialize, Serialize};

/// Nightly sleep summary synced from a wearable device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sleep {
    /// Date in `YYYY-MM-DD` format representing the *morning* of wake-up
    /// (primary key in the DB).
    pub date: String,
    /// Total sleep duration in seconds.
    pub total_seconds: i64,
    /// Time spent in deep (slow-wave) sleep in seconds.
    pub deep_seconds: i64,
    /// Time spent in REM sleep in seconds.
    pub rem_seconds: i64,
    /// Time spent in light sleep in seconds.
    pub light_seconds: i64,
    /// Time spent awake during the sleep period in seconds.
    pub awake_seconds: i64,
    /// Device-reported sleep quality score (0–100), if available.
    pub sleep_score: Option<i32>,
    /// Average HRV during sleep in milliseconds, if reported by the device.
    pub hrv_ms: Option<f64>,
    /// Provider that supplied this record (e.g. `"garmin"`).
    pub source: String,
}
