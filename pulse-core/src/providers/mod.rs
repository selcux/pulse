//! Provider abstraction for syncing health metrics from external platforms.
//!
//! Implement the [`Provider`] trait to add a new data source. Currently
//! supported providers: [`garmin`], [`intervals`].

pub mod garmin;
pub mod intervals;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::db::{Database, queries};

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

/// Summary of a single provider's sync or rebuild run.
#[derive(Debug, Clone)]
pub struct SyncReport {
    /// Human-readable provider name (matches [`Provider::name`]).
    pub provider: String,
    /// Number of mapped records written to the database.
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

/// Keep `raw_responses` in step with a body just received with a 2xx status.
///
/// Stores `body` under `(provider, endpoint, key)` when `enabled` and the body is JSON, replacing
/// any earlier body. Otherwise removes the body stored earlier under that key, so the store never
/// holds a body older than the rows mapped from the latest fetch. Call it before mapping the body.
pub(crate) fn record_raw_response(
    db: &Database,
    enabled: bool,
    provider: &str,
    endpoint: &str,
    key: &str,
    body: &str,
) -> Result<()> {
    let is_json = serde_json::from_str::<serde::de::IgnoredAny>(body).is_ok();
    if enabled && is_json {
        queries::upsert_raw_response(db, provider, endpoint, key, body)
            .with_context(|| format!("Failed to store {provider} {endpoint} response for {key}"))
    } else {
        queries::delete_raw_response(db, provider, endpoint, key).with_context(|| {
            format!("Failed to remove stored {provider} {endpoint} response for {key}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(db: &Database) -> Option<String> {
        queries::get_raw_response(db, "garmin", "sleep", "2026-03-01").unwrap()
    }

    fn record(db: &Database, enabled: bool, body: &str) {
        record_raw_response(db, enabled, "garmin", "sleep", "2026-03-01", body).unwrap();
    }

    #[test]
    fn record_stores_a_json_body_when_on() {
        let db = Database::open_memory().unwrap();
        record(&db, true, r#"{"a":1}"#);
        assert_eq!(stored(&db).as_deref(), Some(r#"{"a":1}"#));
    }

    #[test]
    fn record_replaces_the_previous_body() {
        let db = Database::open_memory().unwrap();
        record(&db, true, r#"{"a":1}"#);
        record(&db, true, r#"{"a":2}"#);
        assert_eq!(stored(&db).as_deref(), Some(r#"{"a":2}"#));
    }

    #[test]
    fn record_when_off_stores_nothing_and_removes_the_previous_body() {
        let db = Database::open_memory().unwrap();
        record(&db, true, r#"{"a":1}"#);
        record(&db, false, r#"{"a":2}"#);
        assert!(stored(&db).is_none());
    }

    #[test]
    fn record_does_not_store_a_body_that_is_not_json() {
        let db = Database::open_memory().unwrap();
        record(&db, true, r#"{"a":1}"#);
        record(&db, true, "");
        assert!(stored(&db).is_none());
        record(&db, true, "<html>");
        assert!(stored(&db).is_none());
    }
}
