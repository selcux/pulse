use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub date: String,
    pub steps: Option<i32>,
    pub active_minutes: Option<i32>,
    pub floors: Option<i32>,
    pub source: String,
}
