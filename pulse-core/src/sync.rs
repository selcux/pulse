use anyhow::{Context, Result};

use crate::config::PulseConfig;
use crate::date_range::DateRange;
use crate::db::Database;
use crate::providers::garmin::{self, GarminProvider};
use crate::providers::intervals::{self, IntervalsProvider};
use crate::providers::{Provider, SyncReport};

/// Result of syncing all providers.
pub struct FullSyncReport {
    pub reports: Vec<SyncReport>,
    pub total_synced: usize,
    pub total_errors: usize,
}

/// Sync all enabled providers for the dates in `range`.
pub fn sync_all(db: &Database, config: &PulseConfig, range: &DateRange) -> Result<FullSyncReport> {
    let providers = build_providers(config);

    let mut reports = Vec::new();
    let mut total_synced = 0;
    let mut total_errors = 0;

    for provider in &providers {
        match provider.sync(db, range) {
            Ok(report) => {
                total_synced += report.records_synced;
                total_errors += report.errors.len();
                reports.push(report);
            }
            Err(e) => {
                reports.push(SyncReport {
                    provider: provider.name().to_string(),
                    records_synced: 0,
                    errors: vec![e.to_string()],
                });
                total_errors += 1;
            }
        }
    }

    Ok(FullSyncReport {
        reports,
        total_synced,
        total_errors,
    })
}

/// Sync a specific provider by name, for the dates in `range`.
pub fn sync_provider(
    db: &Database,
    config: &PulseConfig,
    provider_name: &str,
    range: &DateRange,
) -> Result<SyncReport> {
    let providers = build_providers(config);

    let provider = providers
        .iter()
        .find(|p| p.name() == provider_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown provider: {}", provider_name))?;

    provider.sync(db, range)
}

/// Refill the mapped tables from the provider responses stored in `raw_responses`, without network access.
///
/// Runs in one transaction: a database error rolls the whole rebuild back. A stored body that no
/// longer parses is reported in its provider's `errors` and leaves its rows as they were. Rows synced
/// before raw storage existed have no stored body and are left as they are. Ignores `enabled` in the config.
pub fn rebuild_all(db: &Database) -> Result<FullSyncReport> {
    // Dropping the transaction without a commit rolls it back.
    let tx = db
        .conn()
        .unchecked_transaction()
        .context("Failed to start rebuild")?;

    let reports = vec![garmin::rebuild(db)?, intervals::rebuild(db)?];

    tx.commit().context("Failed to commit rebuild")?;

    let total_synced = reports.iter().map(|r| r.records_synced).sum();
    let total_errors = reports.iter().map(|r| r.errors.len()).sum();
    Ok(FullSyncReport {
        reports,
        total_synced,
        total_errors,
    })
}

fn build_providers(config: &PulseConfig) -> Vec<Box<dyn Provider>> {
    let mut providers: Vec<Box<dyn Provider>> = Vec::new();

    if let Some(ref garmin) = config.providers.garmin
        && garmin.enabled
    {
        providers.push(Box::new(
            GarminProvider::new().with_raw_responses(config.storage.raw_responses),
        ));
    }

    if let Some(ref intervals) = config.providers.intervals
        && intervals.enabled
    {
        providers.push(Box::new(
            IntervalsProvider::new(intervals.clone())
                .with_raw_responses(config.storage.raw_responses),
        ));
    }

    providers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        GarminConfig, IntervalsConfig, ProvidersConfig, PulseConfig, StorageConfig, UserConfig,
    };
    use crate::db::queries;

    fn test_config(garmin_enabled: bool, intervals: Option<IntervalsConfig>) -> PulseConfig {
        PulseConfig {
            user: UserConfig {
                age: None,
                sleep_target_hours: Some(8.0),
                steps_target: Some(10_000),
                rhr_baseline: None,
                hrv_baseline: None,
                vo2_max: None,
                lean_body_mass_kg: None,
                height_cm: None,
            },
            providers: ProvidersConfig {
                garmin: Some(GarminConfig {
                    enabled: garmin_enabled,
                    username: None,
                }),
                intervals,
            },
            storage: StorageConfig::default(),
        }
    }

    #[test]
    fn build_providers_nothing_enabled() {
        let config = test_config(false, None);
        let providers = build_providers(&config);
        assert!(providers.is_empty());
    }

    #[test]
    fn build_providers_with_intervals_enabled() {
        let intervals_cfg = IntervalsConfig {
            enabled: true,
            api_key: "test-key".into(),
            athlete_id: "123".into(),
        };
        let config = test_config(false, Some(intervals_cfg));
        let providers = build_providers(&config);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name(), "intervals");
    }

    #[test]
    fn build_providers_with_garmin_enabled() {
        let config = test_config(true, None);
        let providers = build_providers(&config);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name(), "garmin");
    }

    #[test]
    fn build_providers_with_both_enabled() {
        let intervals_cfg = IntervalsConfig {
            enabled: true,
            api_key: "test-key".into(),
            athlete_id: "123".into(),
        };
        let config = test_config(true, Some(intervals_cfg));
        let providers = build_providers(&config);
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0].name(), "garmin");
        assert_eq!(providers[1].name(), "intervals");
    }

    #[test]
    fn sync_all_with_no_providers_returns_empty_report() {
        let db = crate::db::Database::open_memory().unwrap();
        let config = test_config(false, None);
        let range = DateRange::days_ending("2026-03-01".parse().unwrap(), 7);
        let report = sync_all(&db, &config, &range).unwrap();
        assert!(report.reports.is_empty());
        assert_eq!(report.total_synced, 0);
        assert_eq!(report.total_errors, 0);
    }

    const SLEEP: &str = r#"{"dailySleepDTO":{"sleepTimeInSeconds":28800,"deepSleepSeconds":7200,"remSleepSeconds":5400,"lightSleepSeconds":14400,"awakeSleepSeconds":1800}}"#;
    const SUMMARY: &str = r#"{"restingHeartRate":55,"totalSteps":12345}"#;
    const RUN: &str = r#"{"id":"i1","start_date_local":"2026-03-01T08:00:00","type":"Run"}"#;

    #[test]
    fn rebuild_all_on_an_empty_store_writes_nothing() {
        let db = crate::db::Database::open_memory().unwrap();
        let report = rebuild_all(&db).unwrap();
        let names: Vec<&str> = report.reports.iter().map(|r| r.provider.as_str()).collect();
        assert_eq!(names, ["garmin", "intervals"]);
        assert_eq!(report.total_synced, 0);
        assert_eq!(report.total_errors, 0);
    }

    #[test]
    fn rebuild_all_refills_both_providers() {
        let db = crate::db::Database::open_memory().unwrap();
        queries::upsert_raw_response(&db, "garmin", "sleep", "2026-03-01", SLEEP).unwrap();
        queries::upsert_raw_response(&db, "garmin", "daily_summary", "2026-03-01", SUMMARY)
            .unwrap();
        queries::upsert_raw_response(&db, "intervals", "activity", "i1", RUN).unwrap();
        queries::upsert_raw_response(
            &db,
            "intervals",
            "wellness",
            "2026-03-01",
            r#"{"id":"2026-03-01","ctl":40.5,"atl":45.0}"#,
        )
        .unwrap();

        let report = rebuild_all(&db).unwrap();

        assert_eq!(report.total_errors, 0);
        // 1 sleep, 4 daily rows, 1 workout, 1 wellness day.
        assert_eq!(report.total_synced, 7);
    }

    #[test]
    fn rebuild_all_fills_vo2_max_for_the_baseline_and_vitality() {
        let db = crate::db::Database::open_memory().unwrap();
        let day: chrono::NaiveDate = "2026-03-01".parse().unwrap();
        let today = day.to_string();
        let maxmet = format!(
            r#"[{{"userId":1,"generic":{{"calendarDate":"{today}","vo2MaxPreciseValue":47.6,"vo2MaxValue":48.0}},"cycling":null}}]"#
        );
        queries::upsert_raw_response(&db, "garmin", "sleep", &today, SLEEP).unwrap();
        queries::upsert_raw_response(&db, "garmin", "daily_summary", &today, SUMMARY).unwrap();
        queries::upsert_raw_response(&db, "garmin", "maxmet", &today, &maxmet).unwrap();

        let report = rebuild_all(&db).unwrap();

        assert_eq!(report.total_errors, 0);
        let baseline = queries::compute_vo2_baseline(&db, &DateRange::days_ending(day, 30))
            .unwrap()
            .unwrap();
        assert!((baseline - 47.6).abs() < 1e-9);
        let scores = crate::vitality::calculate_vitality(
            &db,
            &test_config(false, None),
            &DateRange::days_ending(day, 7),
            day,
        )
        .unwrap();
        assert_eq!(scores.len(), 1);
        // vo2_population_score(47.6) = 71.4 + 0.6 / 5 * 14.3 = 73.116
        assert!((scores[0].fitness_score.unwrap() - 73.116).abs() < 0.01);
    }

    #[test]
    fn rebuild_all_rolls_back_on_a_database_error() {
        let db = crate::db::Database::open_memory().unwrap();
        queries::upsert_raw_response(&db, "garmin", "sleep", "2026-03-01", SLEEP).unwrap();
        queries::upsert_raw_response(&db, "garmin", "daily_summary", "2026-03-01", SUMMARY)
            .unwrap();
        // Sleep is written, then the heart upsert fails.
        db.conn().execute_batch("DROP TABLE heart;").unwrap();

        assert!(rebuild_all(&db).is_err());

        assert!(db.conn().is_autocommit());
        assert!(queries::get_sleep(&db, "2026-03-01").unwrap().is_none());
    }

    #[test]
    fn sync_provider_unknown_name_errors() {
        let db = crate::db::Database::open_memory().unwrap();
        let config = test_config(false, None);
        let range = DateRange::days_ending("2026-03-01".parse().unwrap(), 7);
        let result = sync_provider(&db, &config, "nonexistent", &range);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown provider"));
    }
}
