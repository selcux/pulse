use anyhow::Result;
use chrono::Local;

use crate::config::PulseConfig;
use crate::db::Database;
use crate::providers::garmin::GarminProvider;
use crate::providers::intervals::IntervalsProvider;
use crate::providers::{DateRange, Provider, SyncReport};

/// Result of syncing all providers.
pub struct FullSyncReport {
    pub reports: Vec<SyncReport>,
    pub total_synced: usize,
    pub total_errors: usize,
}

/// Sync all enabled providers for the given number of days.
pub fn sync_all(db: &Database, config: &PulseConfig, days: u32) -> Result<FullSyncReport> {
    let range = date_range(days);
    let providers = build_providers(config);

    let mut reports = Vec::new();
    let mut total_synced = 0;
    let mut total_errors = 0;

    for provider in &providers {
        match provider.sync(db, &range) {
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

/// Sync a specific provider by name.
pub fn sync_provider(
    db: &Database,
    config: &PulseConfig,
    provider_name: &str,
    days: u32,
) -> Result<SyncReport> {
    let range = date_range(days);
    let providers = build_providers(config);

    let provider = providers
        .iter()
        .find(|p| p.name() == provider_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown provider: {}", provider_name))?;

    provider.sync(db, &range)
}

fn date_range(days: u32) -> DateRange {
    let end = Local::now().date_naive();
    let start = end - chrono::Duration::days(days as i64);
    DateRange { start, end }
}

fn build_providers(config: &PulseConfig) -> Vec<Box<dyn Provider>> {
    let mut providers: Vec<Box<dyn Provider>> = Vec::new();

    if let Some(ref garmin) = config.providers.garmin {
        if garmin.enabled {
            providers.push(Box::new(GarminProvider::new()));
        }
    }

    if let Some(ref intervals) = config.providers.intervals {
        if intervals.enabled {
            providers.push(Box::new(IntervalsProvider::new(intervals.clone())));
        }
    }

    providers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        GarminConfig, IntervalsConfig, ProvidersConfig, PulseConfig, UserConfig,
    };

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
    fn date_range_produces_correct_span() {
        let range = date_range(7);
        let diff = range.end - range.start;
        assert_eq!(diff.num_days(), 7);
        assert_eq!(range.end, Local::now().date_naive());
    }

    #[test]
    fn sync_all_with_no_providers_returns_empty_report() {
        let db = crate::db::Database::open_memory().unwrap();
        let config = test_config(false, None);
        let report = sync_all(&db, &config, 7).unwrap();
        assert!(report.reports.is_empty());
        assert_eq!(report.total_synced, 0);
        assert_eq!(report.total_errors, 0);
    }

    #[test]
    fn sync_provider_unknown_name_errors() {
        let db = crate::db::Database::open_memory().unwrap();
        let config = test_config(false, None);
        let result = sync_provider(&db, &config, "nonexistent", 7);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown provider"));
    }
}
