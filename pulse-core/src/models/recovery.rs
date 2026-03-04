//! Daily recovery metrics based on Garmin Body Battery.

use serde::{Deserialize, Serialize};

/// Daily recovery status from Garmin Body Battery energy tracking.
///
/// Body Battery is a 0–100 energy reserve score that Garmin computes from
/// HRV, sleep quality, stress, and activity. It charges during rest and
/// drains during stress and physical exertion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recovery {
    /// Date in `YYYY-MM-DD` format (primary key in the DB).
    pub date: String,
    /// Energy charged (recovered) during the day, in Body Battery units.
    pub body_battery_charged: Option<i32>,
    /// Energy drained (used) during the day, in Body Battery units.
    pub body_battery_drained: Option<i32>,
    /// Peak Body Battery level reached during the day (typically after sleep).
    pub body_battery_peak: Option<i32>,
    /// Lowest Body Battery level during the day (typically at end of day).
    pub body_battery_low: Option<i32>,
    /// Provider that supplied this record (e.g. `"garmin"`).
    pub source: String,
}
