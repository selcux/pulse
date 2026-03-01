use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stress {
    pub date: String,
    pub avg_stress: Option<i32>,
    pub max_stress: Option<i32>,
    pub source: String,
}
