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
    /// When the sleep period started, in the device's local wall-clock time as `YYYY-MM-DD HH:MM:SS`
    /// (no offset). Usually the evening before `date`.
    pub start_time: Option<String>,
    /// When the sleep period ended (wake-up), local wall-clock time as `YYYY-MM-DD HH:MM:SS`.
    pub end_time: Option<String>,
    /// Average blood oxygen saturation (SpO2) during sleep, in percent. Only recorded when pulse ox
    /// tracking is on.
    pub spo2_avg: Option<f64>,
    /// Average respiration rate during sleep, in breaths per minute.
    pub respiration_avg: Option<f64>,
    /// Provider that supplied this record (e.g. `"garmin"`).
    pub source: String,
}
