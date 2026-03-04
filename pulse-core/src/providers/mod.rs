//! Provider abstraction for syncing health metrics from external platforms.
//!
//! Implement the [`Provider`] trait to add a new data source. Currently
//! supported providers: [`garmin`], [`intervals`].

pub mod garmin;
pub mod intervals;

use anyhow::Result;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::db::Database;

/// Identifies a category of health metric that a provider can supply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetricType {
    /// Nightly sleep summary (stages, score, HRV).
    Sleep,
    /// Heart rate and cardiovascular metrics (RHR, HRV, VO2 Max).
    Heart,
    /// Garmin Body Battery recovery data.
    Recovery,
    /// Daily activity summary (steps, active minutes, floors).
    Activity,
    /// HRV-based stress levels.
    Stress,
    /// Workout sessions and exercise sets.
    Workout,
}

/// An inclusive date range used to scope sync operations.
#[derive(Debug, Clone)]
pub struct DateRange {
    /// First date to sync (inclusive).
    pub start: NaiveDate,
    /// Last date to sync (inclusive).
    pub end: NaiveDate,
}

/// Summary of a single provider's sync run.
#[derive(Debug, Clone)]
pub struct SyncReport {
    /// Human-readable provider name (matches [`Provider::name`]).
    pub provider: String,
    /// Total number of records written to the database.
    pub records_synced: usize,
    /// Non-fatal error messages collected during the sync.
    pub errors: Vec<String>,
}

/// Trait implemented by each health data provider.
///
/// A provider fetches metric data for a date range and persists it to the
/// local [`Database`]. Errors that affect individual days should be collected
/// into [`SyncReport::errors`] rather than bubbling up as `Err`.
pub trait Provider {
    /// Short, stable identifier for this provider (e.g. `"garmin"`, `"intervals"`).
    fn name(&self) -> &str;

    /// Fetch all supported metrics for `range` and write them to `db`.
    fn sync(&self, db: &Database, range: &DateRange) -> Result<SyncReport>;

    /// The subset of [`MetricType`]s this provider can supply.
    fn supported_metrics(&self) -> &[MetricType];
}
