use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recovery {
    pub date: String,
    pub body_battery_charged: Option<i32>,
    pub body_battery_drained: Option<i32>,
    pub body_battery_peak: Option<i32>,
    pub body_battery_low: Option<i32>,
    pub source: String,
}
