//! Composite vitality scoring and pace-of-aging trend calculation.
//!
//! [`calculate_vitality`] produces a [`VitalityScore`] for each day that has
//! sleep data (sleep is the "anchor" metric). [`calculate_pace`] derives a
//! trend from a slice of scored days, indicating whether recent vitality is
//! above or below the longer-term baseline.

use anyhow::Result;
use serde::Serialize;

use crate::config::PulseConfig;
use crate::db::queries;
use crate::db::Database;

/// Composite vitality score for a single day.
///
/// `total_score` is a weighted blend of the component scores.
/// Weights differ depending on whether a `fitness_score` is available —
/// see [`calculate_vitality`] for the exact formula.
#[derive(Debug, Clone, Serialize)]
pub struct VitalityScore {
    pub date: String,
    pub total_score: f64,           // 0-100
    pub sleep_score: f64,           // 0-100 (weight: 30% without fitness, 25% with)
    pub recovery_score: f64,        // 0-100 (weight: 25% without fitness, 20% with)
    pub activity_score: f64,        // 0-100 (weight: 25% without fitness, 20% with)
    pub stress_score: f64,          // 0-100 (weight: 20% without fitness, 15% with)
    pub fitness_score: Option<f64>, // 0-100 (weight: 20% when present)
}

/// Direction of recent vitality relative to the longer-term baseline.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaceTrend {
    /// 7-day average is ≥10% above the 30-day average (improving).
    Slowing,
    /// 7-day average is within 10% of the 30-day average (stable).
    Steady,
    /// 7-day average is >10% below the 30-day average (declining).
    Accelerating,
}

/// Pace-of-aging trend computed from a rolling window of vitality scores.
#[derive(Debug, Clone, Serialize)]
pub struct PaceInfo {
    pub multiplier: f64,     // 7d_avg / 30d_avg
    pub trend: PaceTrend,
    pub vs_baseline: f64,    // 7d_avg - 30d_avg (signed)
    pub seven_day_avg: f64,
    pub thirty_day_avg: f64,
}

/// Compute pace-of-aging from a slice of scored days (newest-first).
/// Returns None if fewer than 7 days are present.
pub fn calculate_pace(scores: &[VitalityScore]) -> Option<PaceInfo> {
    if scores.len() < 7 {
        return None;
    }
    let seven_day_avg = scores[..7].iter().map(|s| s.total_score).sum::<f64>() / 7.0;
    let thirty_day_avg = scores.iter().map(|s| s.total_score).sum::<f64>() / scores.len() as f64;
    if thirty_day_avg == 0.0 {
        return None;
    }
    let multiplier = seven_day_avg / thirty_day_avg;
    let vs_baseline = seven_day_avg - thirty_day_avg;
    let trend = if multiplier >= 1.1 {
        PaceTrend::Slowing
    } else if multiplier >= 0.9 {
        PaceTrend::Steady
    } else {
        PaceTrend::Accelerating
    };
    Some(PaceInfo { multiplier, trend, vs_baseline, seven_day_avg, thirty_day_avg })
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

    // Compute 30-day heart baselines from DB, with config overrides and hardcoded fallbacks.
    let auto_baselines = queries::compute_heart_baselines(db)?;
    let rhr_baseline = config
        .user
        .rhr_baseline
        .or(auto_baselines.rhr_avg)
        .unwrap_or(60.0);
    let hrv_baseline = config
        .user
        .hrv_baseline
        .or(auto_baselines.hrv_avg)
        .unwrap_or(50.0);

    // VO2 Max baseline: 30-day DB avg (live Garmin data) > config fallback > None (excluded)
    let vo2_baseline = queries::compute_vo2_baseline(db)?.or(config.user.vo2_max);

    // Build scores for each date that has sleep data (sleep is the anchor).
    let mut scores = Vec::new();

    for sleep in &sleep_data {
        let date = &sleep.date;

        // Sleep score (30%): based on total sleep vs target + sleep_score from device
        let sleep_component = calculate_sleep_score(sleep, sleep_target);

        // Recovery score (25%): body battery peak + HRV + RHR (relative to baselines)
        let recovery = recovery_data.iter().find(|r| r.date == *date);
        let heart = heart_data.iter().find(|h| h.date == *date);
        let recovery_component =
            calculate_recovery_score(recovery, heart, rhr_baseline, hrv_baseline);

        // Activity score (25%): steps vs target
        let activity = activity_data.iter().find(|a| a.date == *date);
        let activity_component = calculate_activity_score(activity, steps_target);

        // Stress score (20%/15%): inverse -- lower stress = higher score
        let stress = stress_data.iter().find(|s| s.date == *date);
        let stress_component = calculate_stress_score(stress);

        // Fitness score (20% when present): VO2 Max + lean body mass
        // vo2_baseline used as fallback when day has no VO2 reading (Garmin doesn't record daily)
        let day_vo2 = heart.and_then(|h| h.vo2_max).or(vo2_baseline);
        let fitness_component =
            calculate_fitness_score(day_vo2, config.user.lean_body_mass_kg, config.user.height_cm);

        let total = compute_total_score(
            sleep_component,
            recovery_component,
            activity_component,
            stress_component,
            fitness_component,
        );

        scores.push(VitalityScore {
            date: date.clone(),
            total_score: total,
            sleep_score: sleep_component,
            recovery_score: recovery_component,
            activity_score: activity_component,
            stress_score: stress_component,
            fitness_score: fitness_component,
        });
    }

    Ok(scores)
}

/// Piecewise linear VO2 score against Shvartz & Reinbold (1990) population norms for men.
/// Boundaries match the Finnish fitness classification (Urheiluhallit / Heikko…Erinomainen).
fn vo2_population_score(vo2: f64) -> f64 {
    const NORMS: &[(f64, f64)] = &[
        (0.0,   0.0),
        (26.0,  14.3),
        (32.0,  28.6),
        (36.0,  42.9),
        (42.0,  57.1),
        (47.0,  71.4),
        (52.0,  85.7),
        (60.0, 100.0),
    ];
    for w in NORMS.windows(2) {
        let (v0, s0) = w[0];
        let (v1, s1) = w[1];
        if vo2 <= v1 {
            return s0 + (vo2 - v0) / (v1 - v0) * (s1 - s0);
        }
    }
    100.0
}

fn calculate_fitness_score(
    vo2: Option<f64>,
    lbm_kg: Option<f64>,
    height_cm: Option<f64>,
) -> Option<f64> {
    let mut components = Vec::new();

    if let Some(v) = vo2 {
        components.push(vo2_population_score(v));
    }

    if let Some(lbm) = lbm_kg {
        let lbm_score = if let Some(h) = height_cm {
            // FFMI = lbm_kg / height_m²; natural athlete ceiling ≈ 25
            let height_m = h / 100.0;
            let ffmi = lbm / (height_m * height_m);
            (ffmi / 25.0 * 100.0).clamp(0.0, 100.0)
        } else {
            // Fallback: flat 70 kg reference
            (lbm / 70.0 * 100.0).clamp(0.0, 100.0)
        };
        components.push(lbm_score);
    }

    if components.is_empty() {
        None
    } else {
        Some(components.iter().sum::<f64>() / components.len() as f64)
    }
}

fn compute_total_score(
    sleep: f64,
    recovery: f64,
    activity: f64,
    stress: f64,
    fitness: Option<f64>,
) -> f64 {
    match fitness {
        Some(f) => (sleep * 0.25 + recovery * 0.20 + activity * 0.20 + stress * 0.15 + f * 0.20)
            .clamp(0.0, 100.0),
        None => (sleep * 0.30 + recovery * 0.25 + activity * 0.25 + stress * 0.20)
            .clamp(0.0, 100.0),
    }
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
    rhr_baseline: f64,
    hrv_baseline: f64,
) -> f64 {
    let mut components = Vec::new();

    // Body battery peak (0-100 scale already).
    if let Some(r) = recovery {
        if let Some(peak) = r.body_battery_peak {
            components.push(peak as f64);
        }
    }

    if let Some(h) = heart {
        // HRV score: ratio to personal baseline (higher is better).
        if let Some(hrv) = h.hrv_avg {
            components.push((hrv / hrv_baseline * 100.0).clamp(0.0, 100.0));
        }

        // RHR score: lower is better relative to baseline.
        // At baseline -> 100, 10% above baseline -> ~90.
        if let Some(rhr) = h.resting_hr {
            components.push(((2.0 - rhr as f64 / rhr_baseline) * 100.0).clamp(0.0, 100.0));
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
                rhr_baseline: None,
                hrv_baseline: None,
                vo2_max: None,
                lean_body_mass_kg: None,
                height_cm: None,
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
    fn recovery_score_with_battery_hrv_and_rhr() {
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
            resting_hr: Some(60),  // at baseline -> RHR score 100
            max_hr: None,
            min_hr: None,
            hrv_avg: Some(45.0), // 45/50*100 = 90
            vo2_max: None,
            source: "garmin".into(),
        };
        let score = calculate_recovery_score(Some(&recovery), Some(&heart), 60.0, 50.0);
        // avg(80, 90, 100) = 90
        assert!(
            (score - 90.0).abs() < 0.01,
            "Expected ~90.0, got {score}"
        );
    }

    #[test]
    fn recovery_score_no_data_returns_neutral() {
        let score = calculate_recovery_score(None, None, 60.0, 50.0);
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
        let score = calculate_recovery_score(Some(&recovery), None, 60.0, 50.0);
        assert!(
            (score - 90.0).abs() < f64::EPSILON,
            "Expected 90.0, got {score}"
        );
    }

    #[test]
    fn rhr_at_baseline_scores_100() {
        let heart = Heart {
            date: "2026-03-01".into(),
            resting_hr: Some(55),
            max_hr: None,
            min_hr: None,
            hrv_avg: None,
            vo2_max: None,
            source: "garmin".into(),
        };
        let score = calculate_recovery_score(None, Some(&heart), 55.0, 50.0);
        // RHR only: (2.0 - 55/55) * 100 = 100
        assert!(
            (score - 100.0).abs() < 0.01,
            "Expected 100.0, got {score}"
        );
    }

    #[test]
    fn rhr_above_baseline_scores_lower() {
        let heart = Heart {
            date: "2026-03-01".into(),
            resting_hr: Some(66), // 10% above baseline of 60
            max_hr: None,
            min_hr: None,
            hrv_avg: None,
            vo2_max: None,
            source: "garmin".into(),
        };
        let score = calculate_recovery_score(None, Some(&heart), 60.0, 50.0);
        // (2.0 - 66/60) * 100 = (2.0 - 1.1) * 100 = 90
        assert!(
            (score - 90.0).abs() < 0.01,
            "Expected ~90.0, got {score}"
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
    // Baseline resolution (config override vs auto-computed vs fallback)
    // -----------------------------------------------------------------------

    #[test]
    fn baselines_auto_computed_from_db() {
        let db = crate::db::Database::open_memory().unwrap();
        let config = test_config(); // all baseline fields None
        let today = chrono::Local::now().date_naive().to_string();

        // Insert heart data so auto-baselines are computed.
        queries::upsert_heart(
            &db,
            &Heart {
                date: today.clone(),
                resting_hr: Some(58),
                max_hr: None,
                min_hr: None,
                hrv_avg: Some(55.0),
                vo2_max: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        // Insert sleep (anchor) + the same heart data.
        queries::upsert_sleep(
            &db,
            &Sleep {
                date: today.clone(),
                total_seconds: 28800,
                deep_seconds: 0,
                rem_seconds: 0,
                light_seconds: 0,
                awake_seconds: 0,
                sleep_score: None,
                hrv_ms: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        let scores = calculate_vitality(&db, &config, 7).unwrap();
        assert_eq!(scores.len(), 1);

        // With auto-baselines rhr=58, hrv=55 and today's values equal to those:
        // HRV: 55/55*100 = 100, RHR: (2-58/58)*100 = 100 -> avg = 100
        assert!(
            (scores[0].recovery_score - 100.0).abs() < 0.01,
            "Expected 100.0 (at baseline), got {}",
            scores[0].recovery_score
        );
    }

    #[test]
    fn config_baseline_overrides_auto_computed() {
        let db = crate::db::Database::open_memory().unwrap();
        let mut config = test_config();
        config.user.rhr_baseline = Some(50.0); // override: lower baseline
        config.user.hrv_baseline = Some(70.0); // override: higher baseline

        let today = chrono::Local::now().date_naive().to_string();

        queries::upsert_heart(
            &db,
            &Heart {
                date: today.clone(),
                resting_hr: Some(55), // above config baseline of 50
                max_hr: None,
                min_hr: None,
                hrv_avg: Some(56.0), // below config baseline of 70
                vo2_max: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        queries::upsert_sleep(
            &db,
            &Sleep {
                date: today.clone(),
                total_seconds: 28800,
                deep_seconds: 0,
                rem_seconds: 0,
                light_seconds: 0,
                awake_seconds: 0,
                sleep_score: None,
                hrv_ms: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        let scores = calculate_vitality(&db, &config, 7).unwrap();
        // Config baselines used: rhr=50, hrv=70
        // HRV: 56/70*100 = 80, RHR: (2-55/50)*100 = (2-1.1)*100 = 90 -> avg = 85
        assert!(
            (scores[0].recovery_score - 85.0).abs() < 0.01,
            "Expected ~85.0, got {}",
            scores[0].recovery_score
        );
    }

    #[test]
    fn fallback_to_hardcoded_defaults_when_no_db_and_no_config() {
        let db = crate::db::Database::open_memory().unwrap();
        let config = test_config(); // all baseline fields None
        let today = chrono::Local::now().date_naive().to_string();

        // Insert heart data for today only (no 30-day history that differs).
        queries::upsert_heart(
            &db,
            &Heart {
                date: today.clone(),
                resting_hr: Some(60),
                max_hr: None,
                min_hr: None,
                hrv_avg: Some(50.0),
                vo2_max: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        queries::upsert_sleep(
            &db,
            &Sleep {
                date: today.clone(),
                total_seconds: 28800,
                deep_seconds: 0,
                rem_seconds: 0,
                light_seconds: 0,
                awake_seconds: 0,
                sleep_score: None,
                hrv_ms: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        // Auto-baselines from the 1 day of data: rhr=60, hrv=50 (same as hardcoded fallbacks).
        // At baseline: HRV=100, RHR=100 -> avg=100
        let scores = calculate_vitality(&db, &config, 7).unwrap();
        assert!(
            (scores[0].recovery_score - 100.0).abs() < 0.01,
            "Expected 100.0, got {}",
            scores[0].recovery_score
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
                vo2_max: None,
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

        // Recovery: only 1 day of heart data, so auto-baselines = rhr=55, hrv=60.
        // body_battery_peak=80, hrv=60/60*100=100, rhr=(2-55/55)*100=100 -> avg(80,100,100)=93.33
        assert!(
            (s.recovery_score - 93.33).abs() < 0.01,
            "Recovery: expected ~93.33, got {}",
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

        // Total: 90*0.30 + 93.33*0.25 + 100*0.25 + 75*0.20
        //      = 27 + 23.33 + 25 + 15 = 90.33
        let expected_total = 90.0 * 0.30 + 93.33 * 0.25 + 100.0 * 0.25 + 75.0 * 0.20;
        assert!(
            (s.total_score - expected_total).abs() < 0.1,
            "Total: expected ~{expected_total}, got {}",
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

    // -----------------------------------------------------------------------
    // calculate_pace
    // -----------------------------------------------------------------------

    fn make_score(total: f64) -> VitalityScore {
        VitalityScore {
            date: "2026-01-01".into(),
            total_score: total,
            sleep_score: 0.0,
            recovery_score: 0.0,
            activity_score: 0.0,
            stress_score: 0.0,
            fitness_score: None,
        }
    }

    #[test]
    fn pace_returns_none_with_fewer_than_7_scores() {
        let scores: Vec<VitalityScore> = (0..6).map(|_| make_score(70.0)).collect();
        assert!(calculate_pace(&scores).is_none());
    }

    #[test]
    fn pace_returns_some_with_7_or_more_scores() {
        let scores: Vec<VitalityScore> = (0..7).map(|_| make_score(70.0)).collect();
        assert!(calculate_pace(&scores).is_some());
    }

    #[test]
    fn pace_slowing_when_recent_above_baseline() {
        // 7 recent days at 77, 3 older days at 50 → 7d_avg=77, 30d_avg=(77*7+50*3)/10=68.9
        let mut scores: Vec<VitalityScore> = (0..7).map(|_| make_score(77.0)).collect();
        scores.extend((0..3).map(|_| make_score(50.0)));
        let pace = calculate_pace(&scores).unwrap();
        assert!(
            matches!(pace.trend, PaceTrend::Slowing),
            "Expected Slowing, multiplier={:.3}",
            pace.multiplier
        );
        assert!(pace.multiplier >= 1.1);
        assert!(pace.vs_baseline > 0.0, "vs_baseline should be positive");
    }

    #[test]
    fn pace_accelerating_when_recent_below_baseline() {
        // 7 recent days at 50, 3 older days at 80
        let mut scores: Vec<VitalityScore> = (0..7).map(|_| make_score(50.0)).collect();
        scores.extend((0..3).map(|_| make_score(80.0)));
        let pace = calculate_pace(&scores).unwrap();
        assert!(
            matches!(pace.trend, PaceTrend::Accelerating),
            "Expected Accelerating, multiplier={:.3}",
            pace.multiplier
        );
        assert!(pace.multiplier < 0.9);
        assert!(pace.vs_baseline < 0.0, "vs_baseline should be negative");
    }

    #[test]
    fn pace_steady_when_near_baseline() {
        // All 10 days at 70 → multiplier = 1.0
        let scores: Vec<VitalityScore> = (0..10).map(|_| make_score(70.0)).collect();
        let pace = calculate_pace(&scores).unwrap();
        assert!(
            matches!(pace.trend, PaceTrend::Steady),
            "Expected Steady, multiplier={:.3}",
            pace.multiplier
        );
        assert!((pace.multiplier - 1.0).abs() < f64::EPSILON);
        assert!(pace.vs_baseline.abs() < f64::EPSILON);
    }

    #[test]
    fn pace_vs_baseline_is_signed_delta() {
        // 7 recent = 75, 3 older = 60 → 7d_avg=75, 30d_avg=(75*7+60*3)/10=70.5
        let mut scores: Vec<VitalityScore> = (0..7).map(|_| make_score(75.0)).collect();
        scores.extend((0..3).map(|_| make_score(60.0)));
        let pace = calculate_pace(&scores).unwrap();
        let expected_7d = 75.0_f64;
        let expected_30d = (75.0 * 7.0 + 60.0 * 3.0) / 10.0;
        assert!((pace.seven_day_avg - expected_7d).abs() < 0.01);
        assert!((pace.thirty_day_avg - expected_30d).abs() < 0.01);
        assert!((pace.vs_baseline - (expected_7d - expected_30d)).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // vo2_population_score
    // -----------------------------------------------------------------------

    #[test]
    fn vo2_score_at_poor_boundary() {
        // VO2=28.1 is in Heikko (Poor) range: 14.3 + (2.1/6.0)*14.3 ≈ 19.3
        let score = vo2_population_score(28.1);
        assert!((score - 19.3).abs() < 0.2, "Expected ~19.3, got {score}");
    }

    #[test]
    fn vo2_score_excellent() {
        // VO2=52 → boundary score 85.7
        let score = vo2_population_score(52.0);
        assert!((score - 85.7).abs() < 0.01, "Expected 85.7, got {score}");
    }

    #[test]
    fn vo2_score_average() {
        // VO2=39 is in Välttävä–Keskitaso range: 42.9 + (3.0/6.0)*14.2 ≈ 50.0
        let score = vo2_population_score(39.0);
        assert!((score - 50.0).abs() < 0.2, "Expected ~50.0, got {score}");
    }

    #[test]
    fn vo2_score_above_elite_cap() {
        // VO2 above 60 → capped at 100
        let score = vo2_population_score(65.0);
        assert!((score - 100.0).abs() < 0.01, "Expected 100.0 (cap), got {score}");
    }

    // -----------------------------------------------------------------------
    // calculate_fitness_score / compute_total_score
    // -----------------------------------------------------------------------

    #[test]
    fn fitness_score_at_poor_vo2() {
        // VO2=28.1 → ~19.3; no LBM
        let score = calculate_fitness_score(Some(28.1), None, None);
        assert!(score.is_some());
        assert!((score.unwrap() - 19.3).abs() < 0.2, "Expected ~19.3, got {:?}", score);
    }

    #[test]
    fn fitness_score_with_no_data_returns_none() {
        let score = calculate_fitness_score(None, None, None);
        assert!(score.is_none());
    }

    #[test]
    fn fitness_score_lbm_only_no_vo2() {
        // No vo2 → excluded; lbm=70, no height → 70/70*100 = 100
        let score = calculate_fitness_score(None, Some(70.0), None);
        assert!(score.is_some());
        assert!((score.unwrap() - 100.0).abs() < 0.01, "Expected 100.0, got {:?}", score);
    }

    #[test]
    fn fitness_score_lbm_with_height_uses_ffmi() {
        // lbm=72.7, height=183cm → ffmi=72.7/1.83²=21.71 → 21.71/25*100=86.84
        let score = calculate_fitness_score(None, Some(72.7), Some(183.0));
        assert!(score.is_some());
        let s = score.unwrap();
        assert!((s - 86.8).abs() < 0.2, "Expected ~86.8, got {s}");
    }

    #[test]
    fn fitness_score_blends_vo2_and_lbm() {
        // VO2=28.1 → ~19.3; lbm=72.7 height=183 → ~86.8; avg ≈ 53.1
        let score = calculate_fitness_score(Some(28.1), Some(72.7), Some(183.0));
        assert!(score.is_some());
        let s = score.unwrap();
        assert!((s - 53.1).abs() < 0.5, "Expected ~53.1, got {s}");
    }

    #[test]
    fn vitality_total_renormalizes_without_fitness() {
        let total = compute_total_score(80.0, 80.0, 80.0, 80.0, None);
        assert!((total - 80.0).abs() < 0.01);
    }

    #[test]
    fn vitality_total_includes_fitness_when_present() {
        let without = compute_total_score(80.0, 80.0, 80.0, 80.0, None);
        let with_fitness = compute_total_score(80.0, 80.0, 80.0, 80.0, Some(40.0));
        assert!(with_fitness < without, "lower fitness should pull total down");
    }
}
