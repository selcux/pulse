//! Daily wellness: body composition and training load.

use serde::{Deserialize, Serialize};

/// One day of wellness data: weight and body fat when they were measured that day, and the day's
/// fitness and fatigue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wellness {
    /// Date in `YYYY-MM-DD` format (primary key in the DB).
    pub date: String,
    /// Body weight in kilograms from a weigh-in on this day. `None` without one, including when
    /// the provider only carried an earlier weight over to this day.
    pub weight_kg: Option<f64>,
    /// Body fat in percent of body weight, from a measurement on this day.
    pub body_fat_percent: Option<f64>,
    /// Chronic training load ("fitness"): the provider's long-term (typically 42-day) exponentially
    /// weighted average of daily training load. 0 is a value: no load in that window.
    pub ctl: Option<f64>,
    /// Acute training load ("fatigue"): the short-term (typically 7-day) average of the same load.
    pub atl: Option<f64>,
    /// Provider that supplied this record (e.g. `"intervals"`).
    pub source: String,
}
