use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workout {
    pub id: String,
    pub name: Option<String>,
    pub start_time: String,
    pub duration_seconds: i64,
    pub activity_type: String,
    pub calories: Option<i32>,
    pub avg_hr: Option<i32>,
    pub max_hr: Option<i32>,
    pub source: String,
}
