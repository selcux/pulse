//! Daily stress level summary from wearable HRV-based stress tracking.

use serde::{Deserialize, Serialize};

/// Daily stress summary computed from HRV-based stress tracking.
///
/// Garmin stress scores range from 0 (no stress) to 100 (high stress).
/// Typical resting values fall in the 20–30 range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stress {
    /// Date in `YYYY-MM-DD` format (primary key in the DB).
    pub date: String,
    /// Average stress level across all measured minutes (0–100).
    pub avg_stress: Option<i32>,
    /// Peak stress level recorded during the day (0–100).
    pub max_stress: Option<i32>,
    /// Provider that supplied this record (e.g. `"garmin"`).
    pub source: String,
}
