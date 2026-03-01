use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sleep {
    pub date: String,
    pub total_seconds: i64,
    pub deep_seconds: i64,
    pub rem_seconds: i64,
    pub light_seconds: i64,
    pub awake_seconds: i64,
    pub sleep_score: Option<i32>,
    pub hrv_ms: Option<f64>,
    pub source: String,
}
