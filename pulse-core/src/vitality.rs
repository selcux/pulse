use anyhow::Result;
use serde::Serialize;

use crate::config::PulseConfig;
use crate::db::queries;
use crate::db::Database;

#[derive(Debug, Clone, Serialize)]
pub struct VitalityScore {
    pub date: String,
    pub total_score: f64,    // 0-100
    pub sleep_score: f64,    // 0-100 (weight: 30%)
    pub recovery_score: f64, // 0-100 (weight: 25%)
    pub activity_score: f64, // 0-100 (weight: 25%)
    pub stress_score: f64,   // 0-100 (weight: 20%)
}

/// Calculate vitality scores for the last N days.
///
/// Sleep data acts as the anchor — a score is produced for each date that has
/// sleep data.  The remaining metrics are looked up by date and blended using
/// the following weights:
///
/// | Component | Weight |
/// |-----------|--------|
/// | Sleep     | 30%    |
/// | Recovery  | 25%    |
/// | Activity  | 25%    |
/// | Stress    | 20%    |
pub fn calculate_vitality(
    db: &Database,
    config: &PulseConfig,
    days: i32,
) -> Result<Vec<VitalityScore>> {
    let sleep_data = queries::query_sleep(db, days)?;
    let heart_data = queries::query_heart(db, days)?;
    let recovery_data = queries::query_recovery(db, days)?;
    let activity_data = queries::query_activity(db, days)?;
    let stress_data = queries::query_stress(db, days)?;

    let sleep_target = config.user.sleep_target_hours.unwrap_or(8.0);
    let steps_target = config.user.steps_target.unwrap_or(10_000);

    // Build scores for each date that has sleep data (sleep is the anchor).
    let mut scores = Vec::new();

    for sleep in &sleep_data {
        let date = &sleep.date;

        // Sleep score (30%): based on total sleep vs target + sleep_score from device
        let sleep_component = calculate_sleep_score(sleep, sleep_target);

        // Recovery score (25%): body battery peak + HRV
        let recovery = recovery_data.iter().find(|r| r.date == *date);
        let heart = heart_data.iter().find(|h| h.date == *date);
        let recovery_component = calculate_recovery_score(recovery, heart);

        // Activity score (25%): steps vs target
        let activity = activity_data.iter().find(|a| a.date == *date);
        let activity_component = calculate_activity_score(activity, steps_target);

        // Stress score (20%): inverse -- lower stress = higher score
        let stress = stress_data.iter().find(|s| s.date == *date);
        let stress_component = calculate_stress_score(stress);

        let total = sleep_component * 0.30
            + recovery_component * 0.25
            + activity_component * 0.25
            + stress_component * 0.20;

        scores.push(VitalityScore {
            date: date.clone(),
            total_score: total.clamp(0.0, 100.0),
            sleep_score: sleep_component,
            recovery_score: recovery_component,
            activity_score: activity_component,
            stress_score: stress_component,
        });
    }

    Ok(scores)
}

fn calculate_sleep_score(sleep: &crate::models::Sleep, target_hours: f64) -> f64 {
    let target_seconds = target_hours * 3600.0;

    // Duration-based score: ratio of actual to target sleep.
    let duration_score = ((sleep.total_seconds as f64 / target_seconds) * 100.0).clamp(0.0, 100.0);

    // If the device provides a sleep score, blend 50/50 with duration-based score.
    match sleep.sleep_score {
        Some(device_score) => (duration_score + device_score as f64) / 2.0,
        None => duration_score,
    }
}

fn calculate_recovery_score(
    recovery: Option<&crate::models::Recovery>,
    heart: Option<&crate::models::Heart>,
) -> f64 {
    let mut components = Vec::new();

    // Body battery peak (0-100 scale already).
    if let Some(r) = recovery {
        if let Some(peak) = r.body_battery_peak {
            components.push(peak as f64);
        }
    }

    // HRV -- normalize (higher is better, typical range 20-80 ms).
    // Score: clamp(hrv / 60 * 100, 0, 100)
    if let Some(h) = heart {
        if let Some(hrv) = h.hrv_avg {
            components.push((hrv / 60.0 * 100.0).clamp(0.0, 100.0));
        }
    }

    if components.is_empty() {
        50.0 // neutral default when no data
    } else {
        components.iter().sum::<f64>() / components.len() as f64
    }
}

fn calculate_activity_score(activity: Option<&crate::models::Activity>, steps_target: i32) -> f64 {
    match activity {
        Some(a) => match a.steps {
            Some(steps) => ((steps as f64 / steps_target as f64) * 100.0).clamp(0.0, 100.0),
            None => 50.0,
        },
        None => 50.0,
    }
}

fn calculate_stress_score(stress: Option<&crate::models::Stress>) -> f64 {
    // Inverse: lower avg stress = higher score.
    // Garmin stress range: 0-100, typical resting 20-30.
    // Score = 100 - avg_stress  (so stress of 25 -> score of 75)
    match stress {
        Some(s) => match s.avg_stress {
            Some(avg) => (100.0 - avg as f64).clamp(0.0, 100.0),
            None => 50.0,
        },
        None => 50.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProvidersConfig, PulseConfig, UserConfig};
    use crate::db::queries;
    use crate::models::{Activity, Heart, Recovery, Sleep, Stress};

    fn test_config() -> PulseConfig {
        PulseConfig {
            user: UserConfig {
                age: None,
                sleep_target_hours: Some(8.0),
                steps_target: Some(10_000),
            },
            providers: ProvidersConfig {
                garmin: None,
                intervals: None,
            },
        }
    }

    // -----------------------------------------------------------------------
    // Sleep score
    // -----------------------------------------------------------------------

    #[test]
    fn sleep_score_7h_of_8h_target() {
        let sleep = Sleep {
            date: "2026-03-01".into(),
            total_seconds: 25200, // 7 hours
            deep_seconds: 0,
            rem_seconds: 0,
            light_seconds: 0,
            awake_seconds: 0,
            sleep_score: None,
            hrv_ms: None,
            source: "garmin".into(),
        };
        let score = calculate_sleep_score(&sleep, 8.0);
        // 7/8 * 100 = 87.5
        assert!((score - 87.5).abs() < 0.01, "Expected ~87.5, got {score}");
    }

    #[test]
    fn sleep_score_blends_with_device_score() {
        let sleep = Sleep {
            date: "2026-03-01".into(),
            total_seconds: 25200, // 7h -> duration score 87.5
            deep_seconds: 0,
            rem_seconds: 0,
            light_seconds: 0,
            awake_seconds: 0,
            sleep_score: Some(70), // device score
            hrv_ms: None,
            source: "garmin".into(),
        };
        let score = calculate_sleep_score(&sleep, 8.0);
        // (87.5 + 70) / 2 = 78.75
        assert!(
            (score - 78.75).abs() < 0.01,
            "Expected ~78.75, got {score}"
        );
    }

    #[test]
    fn sleep_score_over_target_capped_at_100() {
        let sleep = Sleep {
            date: "2026-03-01".into(),
            total_seconds: 36000, // 10 hours
            deep_seconds: 0,
            rem_seconds: 0,
            light_seconds: 0,
            awake_seconds: 0,
            sleep_score: None,
            hrv_ms: None,
            source: "garmin".into(),
        };
        let score = calculate_sleep_score(&sleep, 8.0);
        assert!(
            (score - 100.0).abs() < 0.01,
            "Expected 100.0 (capped), got {score}"
        );
    }

    // -----------------------------------------------------------------------
    // Recovery score
    // -----------------------------------------------------------------------

    #[test]
    fn recovery_score_with_battery_and_hrv() {
        let recovery = Recovery {
            date: "2026-03-01".into(),
            body_battery_charged: None,
            body_battery_drained: None,
            body_battery_peak: Some(80),
            body_battery_low: None,
            source: "garmin".into(),
        };
        let heart = Heart {
            date: "2026-03-01".into(),
            resting_hr: None,
            max_hr: None,
            min_hr: None,
            hrv_avg: Some(45.0), // 45/60*100 = 75
            source: "garmin".into(),
        };
        let score = calculate_recovery_score(Some(&recovery), Some(&heart));
        // avg(80, 75) = 77.5
        assert!(
            (score - 77.5).abs() < 0.01,
            "Expected ~77.5, got {score}"
        );
    }

    #[test]
    fn recovery_score_no_data_returns_neutral() {
        let score = calculate_recovery_score(None, None);
        assert!(
            (score - 50.0).abs() < f64::EPSILON,
            "Expected 50.0, got {score}"
        );
    }

    #[test]
    fn recovery_score_battery_only() {
        let recovery = Recovery {
            date: "2026-03-01".into(),
            body_battery_charged: None,
            body_battery_drained: None,
            body_battery_peak: Some(90),
            body_battery_low: None,
            source: "garmin".into(),
        };
        let score = calculate_recovery_score(Some(&recovery), None);
        assert!(
            (score - 90.0).abs() < f64::EPSILON,
            "Expected 90.0, got {score}"
        );
    }

    // -----------------------------------------------------------------------
    // Activity score
    // -----------------------------------------------------------------------

    #[test]
    fn activity_score_at_target() {
        let activity = Activity {
            date: "2026-03-01".into(),
            steps: Some(10_000),
            active_minutes: None,
            floors: None,
            source: "garmin".into(),
        };
        let score = calculate_activity_score(Some(&activity), 10_000);
        assert!(
            (score - 100.0).abs() < 0.01,
            "Expected 100.0, got {score}"
        );
    }

    #[test]
    fn activity_score_over_target_capped() {
        let activity = Activity {
            date: "2026-03-01".into(),
            steps: Some(15_000),
            active_minutes: None,
            floors: None,
            source: "garmin".into(),
        };
        let score = calculate_activity_score(Some(&activity), 10_000);
        assert!(
            (score - 100.0).abs() < 0.01,
            "Expected 100.0 (capped), got {score}"
        );
    }

    #[test]
    fn activity_score_half_target() {
        let activity = Activity {
            date: "2026-03-01".into(),
            steps: Some(5_000),
            active_minutes: None,
            floors: None,
            source: "garmin".into(),
        };
        let score = calculate_activity_score(Some(&activity), 10_000);
        assert!((score - 50.0).abs() < 0.01, "Expected 50.0, got {score}");
    }

    #[test]
    fn activity_score_no_data_returns_neutral() {
        let score = calculate_activity_score(None, 10_000);
        assert!(
            (score - 50.0).abs() < f64::EPSILON,
            "Expected 50.0, got {score}"
        );
    }

    // -----------------------------------------------------------------------
    // Stress score
    // -----------------------------------------------------------------------

    #[test]
    fn stress_score_avg_25_gives_75() {
        let stress = Stress {
            date: "2026-03-01".into(),
            avg_stress: Some(25),
            max_stress: None,
            source: "garmin".into(),
        };
        let score = calculate_stress_score(Some(&stress));
        assert!((score - 75.0).abs() < 0.01, "Expected 75.0, got {score}");
    }

    #[test]
    fn stress_score_avg_0_gives_100() {
        let stress = Stress {
            date: "2026-03-01".into(),
            avg_stress: Some(0),
            max_stress: None,
            source: "garmin".into(),
        };
        let score = calculate_stress_score(Some(&stress));
        assert!(
            (score - 100.0).abs() < 0.01,
            "Expected 100.0, got {score}"
        );
    }

    #[test]
    fn stress_score_no_data_returns_neutral() {
        let score = calculate_stress_score(None);
        assert!(
            (score - 50.0).abs() < f64::EPSILON,
            "Expected 50.0, got {score}"
        );
    }

    // -----------------------------------------------------------------------
    // Full calculate_vitality with in-memory DB
    // -----------------------------------------------------------------------

    #[test]
    fn calculate_vitality_full_integration() {
        let db = crate::db::Database::open_memory().unwrap();
        let config = test_config();

        // Use today's date so the query_* functions (which filter by date('now', ...))
        // will include our test data.
        let today = chrono::Local::now().date_naive().to_string();

        // Insert test data for today
        queries::upsert_sleep(
            &db,
            &Sleep {
                date: today.clone(),
                total_seconds: 28800, // 8h = 100% of target
                deep_seconds: 7200,
                rem_seconds: 5400,
                light_seconds: 14400,
                awake_seconds: 1800,
                sleep_score: Some(80),
                hrv_ms: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        queries::upsert_heart(
            &db,
            &Heart {
                date: today.clone(),
                resting_hr: Some(55),
                max_hr: Some(165),
                min_hr: Some(48),
                hrv_avg: Some(60.0), // 60/60*100 = 100
                source: "garmin".into(),
            },
        )
        .unwrap();

        queries::upsert_recovery(
            &db,
            &Recovery {
                date: today.clone(),
                body_battery_charged: Some(60),
                body_battery_drained: Some(40),
                body_battery_peak: Some(80),
                body_battery_low: Some(20),
                source: "garmin".into(),
            },
        )
        .unwrap();

        queries::upsert_activity(
            &db,
            &Activity {
                date: today.clone(),
                steps: Some(10_000), // exactly at target = 100
                active_minutes: Some(60),
                floors: Some(10),
                source: "garmin".into(),
            },
        )
        .unwrap();

        queries::upsert_stress(
            &db,
            &Stress {
                date: today.clone(),
                avg_stress: Some(25), // 100 - 25 = 75
                max_stress: Some(50),
                source: "garmin".into(),
            },
        )
        .unwrap();

        let scores = calculate_vitality(&db, &config, 7).unwrap();
        assert_eq!(scores.len(), 1);

        let s = &scores[0];
        assert_eq!(s.date, today);

        // Sleep: duration_score = 100, device = 80 -> (100+80)/2 = 90
        assert!(
            (s.sleep_score - 90.0).abs() < 0.01,
            "Sleep: expected 90.0, got {}",
            s.sleep_score
        );

        // Recovery: body_battery_peak=80, hrv=60/60*100=100 -> avg(80,100)=90
        assert!(
            (s.recovery_score - 90.0).abs() < 0.01,
            "Recovery: expected 90.0, got {}",
            s.recovery_score
        );

        // Activity: 10000/10000 * 100 = 100
        assert!(
            (s.activity_score - 100.0).abs() < 0.01,
            "Activity: expected 100.0, got {}",
            s.activity_score
        );

        // Stress: 100 - 25 = 75
        assert!(
            (s.stress_score - 75.0).abs() < 0.01,
            "Stress: expected 75.0, got {}",
            s.stress_score
        );

        // Total: 90*0.30 + 90*0.25 + 100*0.25 + 75*0.20
        //      = 27 + 22.5 + 25 + 15 = 89.5
        assert!(
            (s.total_score - 89.5).abs() < 0.01,
            "Total: expected 89.5, got {}",
            s.total_score
        );
    }

    #[test]
    fn calculate_vitality_no_data_returns_empty() {
        let db = crate::db::Database::open_memory().unwrap();
        let config = test_config();
        let scores = calculate_vitality(&db, &config, 7).unwrap();
        assert!(scores.is_empty());
    }
}
