use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heart {
    pub date: String,
    pub resting_hr: Option<i32>,
    pub max_hr: Option<i32>,
    pub min_hr: Option<i32>,
    pub hrv_avg: Option<f64>,
    pub vo2_max: Option<f64>,
    pub source: String,
}
