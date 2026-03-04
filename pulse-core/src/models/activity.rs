//! Daily activity summary (steps, active minutes, floors).

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
    /// Provider that supplied this record (e.g. `"garmin"`).
    pub source: String,
}
