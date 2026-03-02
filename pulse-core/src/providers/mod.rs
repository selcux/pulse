pub mod garmin;
pub mod intervals;

use anyhow::Result;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::db::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetricType {
    Sleep,
    Heart,
    Recovery,
    Activity,
    Stress,
    Workout,
}

#[derive(Debug, Clone)]
pub struct DateRange {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

#[derive(Debug, Clone)]
pub struct SyncReport {
    pub provider: String,
    pub records_synced: usize,
    pub errors: Vec<String>,
}

pub trait Provider {
    fn name(&self) -> &str;
    fn sync(&self, db: &Database, range: &DateRange) -> Result<SyncReport>;
    fn supported_metrics(&self) -> &[MetricType];
}
