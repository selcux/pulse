//! Composite vitality scoring and pace-of-aging trend calculation.
//!
//! [`calculate_vitality`] produces a [`VitalityScore`] for each day that has
//! sleep data (sleep is the "anchor" metric). [`calculate_pace`] compares the
//! scores of the 7 days ending at a date with those of the 30 days ending there,
//! indicating whether recent vitality is above or below that baseline. A day that is not over yet
//! (see [`is_complete`]) has a score but is left out of the pace, the way a day without a score is.

use std::num::NonZeroU32;

use anyhow::Result;
use chrono::NaiveDate;
use serde::Serialize;

use crate::config::PulseConfig;
use crate::date_range::DateRange;
use crate::db::Database;
use crate::db::queries;

/// How many days before the end of the scored range the heart and VO2 max baselines average over.
const BASELINE_DAYS: u32 = 30;

/// The dates the baselines average over: `end` and the [`BASELINE_DAYS`] days before it.
///
/// That is 31 dates, not 30. The 31-day span is known and deliberate for now, pending a separate
/// decision; `--days N` meaning "N dates" (issue #22) does not apply to it, which is why this calls
/// [`DateRange::days_ending`] and not [`DateRange::last_days`].
fn baseline_range(end: NaiveDate) -> DateRange {
    DateRange::days_ending(end, BASELINE_DAYS)
}

/// Dates in the pace's recent window: the end date and the 6 days before it.
const PACE_RECENT_DAYS: NonZeroU32 = NonZeroU32::new(7).unwrap();

/// Dates in the pace's baseline window: the end date and the 29 days before it. One date fewer
/// than [`baseline_range`] on purpose: the 30 matches the `thirty_day_avg` name and the `30d avg`
/// label, and the 31-date span of the heart baselines is a separate, pending decision.
const PACE_BASELINE_DAYS: NonZeroU32 = NonZeroU32::new(30).unwrap();

/// Scored, finished days the recent window needs before there is a pace: 5 of its 7, so up to
/// two nights without sleep data.
const PACE_RECENT_MIN_SCORED: usize = 5;

/// Scored, finished days the baseline window needs before there is a pace: 20 of its 30,
/// two-thirds. After a long gap this keeps pace hidden until the baseline again covers most of a
/// month.
const PACE_BASELINE_MIN_SCORED: usize = 20;

/// The pace's recent window: `DateRange::last_days(end, PACE_RECENT_DAYS)`.
fn pace_recent_range(end: NaiveDate) -> DateRange {
    DateRange::last_days(end, PACE_RECENT_DAYS)
}

/// The pace's baseline window: `DateRange::last_days(end, PACE_BASELINE_DAYS)`.
fn pace_baseline_range(end: NaiveDate) -> DateRange {
    DateRange::last_days(end, PACE_BASELINE_DAYS)
}

/// Whether the day at `date` is over, so its activity and stress cover all of it.
///
/// A day is complete when it is before `today`. This is the only place that rule lives, so a better
/// signal can replace it without touching the pace or the output.
fn is_complete(date: NaiveDate, today: NaiveDate) -> bool {
    date < today
}

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
    /// `false` while the day is not over: its activity and stress cover only part of it, and the
    /// pace leaves the score out. Last field on purpose: TOON and ISON rows are positional.
    pub complete: bool,
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

/// Pace of aging at a date: the mean score of the 7 days ending there against the mean score
/// of the 30 days ending there. See [`calculate_pace`].
#[derive(Debug, Clone, Serialize)]
pub struct PaceInfo {
    /// `seven_day_avg / thirty_day_avg`.
    pub multiplier: f64,
    /// Direction of `multiplier`: see [`PaceTrend`].
    pub trend: PaceTrend,
    /// `seven_day_avg - thirty_day_avg`, signed.
    pub vs_baseline: f64,
    /// Mean score of the scored, finished days among the 7 days ending at the pace's date.
    pub seven_day_avg: f64,
    /// Mean score of the scored, finished days among the 30 days ending at the pace's date, the 7
    /// included.
    pub thirty_day_avg: f64,
}

/// Pace of aging at `end`: the mean score of the 7 days ending at `end` against the mean score
/// of the 30 days ending at `end`, the 7 included.
///
/// The windows do not depend on the range a caller displays. This function scores the 30 days
/// itself with [`calculate_vitality`], so a date gets the same score here as in any range that
/// also ends at `end`. Days without a score (no sleep data) and days that are not over yet (see
/// [`is_complete`] with `today`) are left out of both means and do not count towards the minimums.
/// The windows still end at `end`, so a pace ending today is computed from the finished days only.
/// `Ok(None)` when fewer than 5 of the 7 days or 20 of the 30 days have a finished score.
pub fn calculate_pace(
    db: &Database,
    config: &PulseConfig,
    end: NaiveDate,
    today: NaiveDate,
) -> Result<Option<PaceInfo>> {
    let scores = calculate_vitality(db, config, &pace_baseline_range(end), today)?;
    Ok(pace_from_scores(&scores, end))
}

/// Mean `total_score` of the finished scores dated inside `range`, or `None` when fewer than
/// `min_scored` of them are. A score that is not complete or whose date does not parse as
/// `YYYY-MM-DD` does not count.
fn mean_score_in(scores: &[VitalityScore], range: &DateRange, min_scored: usize) -> Option<f64> {
    let totals: Vec<f64> = scores
        .iter()
        .filter(|s| {
            s.complete
                && s.date
                    .parse::<NaiveDate>()
                    .is_ok_and(|d| range.start <= d && d <= range.end)
        })
        .map(|s| s.total_score)
        .collect();
    if totals.len() < min_scored {
        return None;
    }
    Some(totals.iter().sum::<f64>() / totals.len() as f64)
}

/// Pace at `end` from `scores`, in any order. Scores dated outside the 30-day window and scores
/// that are not complete are ignored.
fn pace_from_scores(scores: &[VitalityScore], end: NaiveDate) -> Option<PaceInfo> {
    let seven_day_avg = mean_score_in(scores, &pace_recent_range(end), PACE_RECENT_MIN_SCORED)?;
    let thirty_day_avg =
        mean_score_in(scores, &pace_baseline_range(end), PACE_BASELINE_MIN_SCORED)?;
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
    Some(PaceInfo {
        multiplier,
        trend,
        vs_baseline,
        seven_day_avg,
        thirty_day_avg,
    })
}

/// Calculate vitality scores for each day in `range` that has sleep data, newest first.
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
///
/// The RHR, HRV and VO2 max baselines are averages over the 30 days ending at `range.end`, so a
/// past range is scored against the baselines of its own time.
///
/// `today` is only used to set [`VitalityScore::complete`] on each score; the core never reads the
/// clock.
pub fn calculate_vitality(
    db: &Database,
    config: &PulseConfig,
    range: &DateRange,
    today: NaiveDate,
) -> Result<Vec<VitalityScore>> {
    let sleep_data = queries::query_sleep(db, range)?;
    let heart_data = queries::query_heart(db, range)?;
    let recovery_data = queries::query_recovery(db, range)?;
    let activity_data = queries::query_activity(db, range)?;
    let stress_data = queries::query_stress(db, range)?;

    let sleep_target = config.user.sleep_target_hours.unwrap_or(8.0);
    let steps_target = config.user.steps_target.unwrap_or(10_000);

    let baseline = baseline_range(range.end);

    // Compute 30-day heart baselines from DB, with config overrides and hardcoded fallbacks.
    let auto_baselines = queries::compute_heart_baselines(db, &baseline)?;
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
    let vo2_baseline = queries::compute_vo2_baseline(db, &baseline)?.or(config.user.vo2_max);

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
        let fitness_component = calculate_fitness_score(
            day_vo2,
            config.user.lean_body_mass_kg,
            config.user.height_cm,
        );

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
            // A date that does not parse cannot be placed before today, so it counts as unfinished.
            complete: date
                .parse::<NaiveDate>()
                .is_ok_and(|d| is_complete(d, today)),
        });
    }

    Ok(scores)
}

/// Piecewise linear VO2 score against Shvartz & Reinbold (1990) population norms for men.
/// Boundaries match the Finnish fitness classification (Urheiluhallit / Heikko…Erinomainen).
fn vo2_population_score(vo2: f64) -> f64 {
    const NORMS: &[(f64, f64)] = &[
        (0.0, 0.0),
        (26.0, 14.3),
        (32.0, 28.6),
        (36.0, 42.9),
        (42.0, 57.1),
        (47.0, 71.4),
        (52.0, 85.7),
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
        None => {
            (sleep * 0.30 + recovery * 0.25 + activity * 0.25 + stress * 0.20).clamp(0.0, 100.0)
        }
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
    if let Some(r) = recovery
        && let Some(peak) = r.body_battery_peak
    {
        components.push(peak as f64);
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
    use crate::config::{ProvidersConfig, PulseConfig, StorageConfig, UserConfig};
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
            storage: StorageConfig::default(),
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
            start_time: None,
            end_time: None,
            spo2_avg: None,
            respiration_avg: None,
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
            start_time: None,
            end_time: None,
            spo2_avg: None,
            respiration_avg: None,
            source: "garmin".into(),
        };
        let score = calculate_sleep_score(&sleep, 8.0);
        // (87.5 + 70) / 2 = 78.75
        assert!((score - 78.75).abs() < 0.01, "Expected ~78.75, got {score}");
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
            start_time: None,
            end_time: None,
            spo2_avg: None,
            respiration_avg: None,
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
            resting_hr: Some(60), // at baseline -> RHR score 100
            max_hr: None,
            min_hr: None,
            hrv_avg: Some(45.0), // 45/50*100 = 90
            vo2_max: None,
            source: "garmin".into(),
        };
        let score = calculate_recovery_score(Some(&recovery), Some(&heart), 60.0, 50.0);
        // avg(80, 90, 100) = 90
        assert!((score - 90.0).abs() < 0.01, "Expected ~90.0, got {score}");
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
        assert!((score - 100.0).abs() < 0.01, "Expected 100.0, got {score}");
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
        assert!((score - 90.0).abs() < 0.01, "Expected ~90.0, got {score}");
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
            total_kcal: None,
            active_kcal: None,
            bmr_kcal: None,
            distance_meters: None,
            source: "garmin".into(),
        };
        let score = calculate_activity_score(Some(&activity), 10_000);
        assert!((score - 100.0).abs() < 0.01, "Expected 100.0, got {score}");
    }

    #[test]
    fn activity_score_over_target_capped() {
        let activity = Activity {
            date: "2026-03-01".into(),
            steps: Some(15_000),
            active_minutes: None,
            floors: None,
            total_kcal: None,
            active_kcal: None,
            bmr_kcal: None,
            distance_meters: None,
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
            total_kcal: None,
            active_kcal: None,
            bmr_kcal: None,
            distance_meters: None,
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
            rest_seconds: None,
            low_seconds: None,
            medium_seconds: None,
            high_seconds: None,
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
            rest_seconds: None,
            low_seconds: None,
            medium_seconds: None,
            high_seconds: None,
            source: "garmin".into(),
        };
        let score = calculate_stress_score(Some(&stress));
        assert!((score - 100.0).abs() < 0.01, "Expected 100.0, got {score}");
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
                start_time: None,
                end_time: None,
                spo2_avg: None,
                respiration_avg: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        let scores = calculate_vitality(&db, &config, &last_week(), last_week().end).unwrap();
        assert_eq!(scores.len(), 1);

        // With auto-baselines rhr=58, hrv=55 and today's values equal to those:
        // HRV: 55/55*100 = 100, RHR: (2-58/58)*100 = 100 -> avg = 100
        assert!(
            (scores[0].recovery_score - 100.0).abs() < 0.01,
            "Expected 100.0 (at baseline), got {}",
            scores[0].recovery_score
        );
    }

    /// The 7 dates ending today, which is what `--days 7` selects.
    fn last_week() -> DateRange {
        DateRange::last_days(
            chrono::Local::now().date_naive(),
            std::num::NonZeroU32::new(7).unwrap(),
        )
    }

    fn date(s: &str) -> chrono::NaiveDate {
        s.parse().unwrap()
    }

    /// A `today` after every date the fixed-date tests use, so all their days are complete.
    fn later() -> chrono::NaiveDate {
        date("2099-01-01")
    }

    /// Insert a heart row for `date` with the given values.
    fn insert_heart_day(
        db: &crate::db::Database,
        date: &str,
        resting_hr: Option<i32>,
        hrv_avg: Option<f64>,
        vo2_max: Option<f64>,
    ) {
        queries::upsert_heart(
            db,
            &Heart {
                date: date.into(),
                resting_hr,
                max_hr: None,
                min_hr: None,
                hrv_avg,
                vo2_max,
                source: "garmin".into(),
            },
        )
        .unwrap();
    }

    #[test]
    fn vitality_scores_only_the_days_in_the_range() {
        let db = crate::db::Database::open_memory().unwrap();
        for day in 1..=10 {
            insert_sleep_hours(&db, &format!("2026-03-{day:02}"), 8);
        }
        let range = DateRange {
            start: date("2026-03-03"),
            end: date("2026-03-05"),
        };

        let scores = calculate_vitality(&db, &test_config(), &range, later()).unwrap();
        let dates: Vec<&str> = scores.iter().map(|s| s.date.as_str()).collect();
        assert_eq!(dates, ["2026-03-05", "2026-03-04", "2026-03-03"]);
    }

    #[test]
    fn baseline_span_is_the_range_end_and_the_30_days_before_it() {
        let baseline = baseline_range(date("2026-03-31"));
        assert_eq!(baseline.start, date("2026-03-01"));
        assert_eq!(baseline.end, date("2026-03-31"));
        assert_eq!((baseline.end - baseline.start).num_days() + 1, 31);
    }

    #[test]
    fn baselines_are_the_30_days_ending_at_the_range_end() {
        let db = crate::db::Database::open_memory().unwrap();
        let day = date("2026-03-31");
        insert_sleep_hours(&db, "2026-03-31", 8);
        // Inside the baseline window (03-01..=03-31), and the scored day itself.
        insert_heart_day(&db, "2026-03-01", Some(60), Some(50.0), None);
        insert_heart_day(&db, "2026-03-31", Some(60), Some(50.0), None);
        // Outside it: a leak would pull the baselines away from 60 bpm / 50 ms.
        insert_heart_day(&db, "2026-02-28", Some(40), Some(110.0), None);
        insert_heart_day(&db, "2026-04-01", Some(40), Some(110.0), None);

        let range = DateRange::days_ending(day, 0);
        let scores = calculate_vitality(&db, &test_config(), &range, later()).unwrap();
        assert_eq!(scores.len(), 1);
        assert!(
            (scores[0].recovery_score - 100.0).abs() < 0.01,
            "Expected 100.0 (at baseline), got {}",
            scores[0].recovery_score
        );
    }

    #[test]
    fn vo2_fallback_uses_the_30_days_ending_at_the_range_end() {
        let db = crate::db::Database::open_memory().unwrap();
        insert_sleep_hours(&db, "2026-03-31", 8);
        insert_heart_day(&db, "2026-03-01", None, None, Some(50.0));
        insert_heart_day(&db, "2026-02-28", None, None, Some(30.0));
        insert_heart_day(&db, "2026-03-31", None, None, None);

        let range = DateRange::days_ending(date("2026-03-31"), 0);
        let scores = calculate_vitality(&db, &test_config(), &range, later()).unwrap();
        assert_eq!(scores.len(), 1);
        let actual = scores[0].fitness_score.expect("fitness score present");
        let expected = vo2_population_score(50.0);
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    /// Insert a sleep row of `hours` hours for `date`, so the day gets a vitality score. With
    /// `test_config()` and no other data the day scores 0.30 × min(hours / 8 × 100, 100) + 35:
    /// 65.0 for 8 h, 50.0 for 4 h.
    fn insert_sleep_hours(db: &crate::db::Database, date: &str, hours: i64) {
        queries::upsert_sleep(
            db,
            &Sleep {
                date: date.into(),
                total_seconds: hours * 3600,
                deep_seconds: 0,
                rem_seconds: 0,
                light_seconds: 0,
                awake_seconds: 0,
                sleep_score: None,
                hrv_ms: None,
                start_time: None,
                end_time: None,
                spo2_avg: None,
                respiration_avg: None,
                source: "garmin".into(),
            },
        )
        .unwrap();
    }

    /// `insert_sleep_hours` on each date `offset` days before `end` (offset 0 is `end`).
    fn insert_sleep_before(
        db: &crate::db::Database,
        end: chrono::NaiveDate,
        offsets: impl IntoIterator<Item = u64>,
        hours: i64,
    ) {
        for offset in offsets {
            let day = end - chrono::Days::new(offset);
            insert_sleep_hours(db, &day.to_string(), hours);
        }
    }

    #[test]
    fn fitness_uses_the_30_day_vo2_average_on_days_without_an_estimate() {
        let db = crate::db::Database::open_memory().unwrap();
        let config = test_config();
        let today = chrono::Local::now().date_naive();
        let days = [
            (today, Some(50.0)),
            (today - chrono::Duration::days(1), None),
            (today - chrono::Duration::days(2), Some(46.0)),
        ];

        for (date, vo2_max) in days {
            let date = date.to_string();
            insert_sleep_hours(&db, &date, 8);
            queries::upsert_heart(
                &db,
                &Heart {
                    date,
                    resting_hr: None,
                    max_hr: None,
                    min_hr: None,
                    hrv_avg: None,
                    vo2_max,
                    source: "garmin".into(),
                },
            )
            .unwrap();
        }

        let scores = calculate_vitality(&db, &config, &last_week(), last_week().end).unwrap();
        assert_eq!(scores.len(), 3);

        // Own estimate on days that have one; the mean (48.0) on the day without.
        for (date, expected_vo2) in days.map(|(d, _)| d).into_iter().zip([50.0, 48.0, 46.0]) {
            let score = scores
                .iter()
                .find(|s| s.date == date.to_string())
                .unwrap_or_else(|| panic!("no score for {date}"));
            let expected = vo2_population_score(expected_vo2);
            let actual = score.fitness_score.expect("fitness score present");
            assert!(
                (actual - expected).abs() < 1e-9,
                "{date}: expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn fitness_falls_back_to_the_config_vo2_max_without_synced_estimates() {
        let db = crate::db::Database::open_memory().unwrap();
        let mut config = test_config();
        config.user.vo2_max = Some(42.0);
        let today = chrono::Local::now().date_naive().to_string();
        insert_sleep_hours(&db, &today, 8);

        let scores = calculate_vitality(&db, &config, &last_week(), last_week().end).unwrap();

        assert_eq!(scores.len(), 1);
        let actual = scores[0].fitness_score.expect("fitness score present");
        let expected = vo2_population_score(42.0);
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
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
                start_time: None,
                end_time: None,
                spo2_avg: None,
                respiration_avg: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        let scores = calculate_vitality(&db, &config, &last_week(), last_week().end).unwrap();
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
                start_time: None,
                end_time: None,
                spo2_avg: None,
                respiration_avg: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        // Auto-baselines from the 1 day of data: rhr=60, hrv=50 (same as hardcoded fallbacks).
        // At baseline: HRV=100, RHR=100 -> avg=100
        let scores = calculate_vitality(&db, &config, &last_week(), last_week().end).unwrap();
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

        // Use today's date so the data falls inside `last_week()`, the range passed below.
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
                start_time: None,
                end_time: None,
                spo2_avg: None,
                respiration_avg: None,
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
                total_kcal: None,
                active_kcal: None,
                bmr_kcal: None,
                distance_meters: None,
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
                rest_seconds: None,
                low_seconds: None,
                medium_seconds: None,
                high_seconds: None,
                source: "garmin".into(),
            },
        )
        .unwrap();

        let scores = calculate_vitality(&db, &config, &last_week(), last_week().end).unwrap();
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
        let scores = calculate_vitality(&db, &config, &last_week(), last_week().end).unwrap();
        assert!(scores.is_empty());
    }

    // -----------------------------------------------------------------------
    // calculate_pace
    // -----------------------------------------------------------------------

    /// One score per day: `totals[i]` is dated `i` days before `end`. Other fields 0.0 / None.
    fn daily_scores(end: chrono::NaiveDate, totals: &[f64]) -> Vec<VitalityScore> {
        totals
            .iter()
            .enumerate()
            .map(|(offset, &total)| VitalityScore {
                date: (end - chrono::Days::new(offset as u64)).to_string(),
                total_score: total,
                sleep_score: 0.0,
                recovery_score: 0.0,
                activity_score: 0.0,
                stress_score: 0.0,
                fitness_score: None,
                complete: true,
            })
            .collect()
    }

    /// `count` copies of `total`, for building `daily_scores` inputs.
    fn repeat_score(total: f64, count: usize) -> Vec<f64> {
        vec![total; count]
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn pace_windows_are_the_7_and_30_dates_ending_at_the_end() {
        let end = date("2026-03-31");

        let recent = pace_recent_range(end);
        assert_eq!(recent.start, date("2026-03-25"));
        assert_eq!(recent.end, end);

        let baseline = pace_baseline_range(end);
        assert_eq!(baseline.start, date("2026-03-02"));
        assert_eq!(baseline.end, end);
        assert_eq!((baseline.end - baseline.start).num_days() + 1, 30);
        assert_eq!(
            (baseline_range(end).end - baseline_range(end).start).num_days() + 1,
            31,
            "the heart baselines keep their 31 dates"
        );
    }

    #[test]
    fn pace_with_a_week_shown_still_uses_30_days() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");
        insert_sleep_before(&db, end, 0..=6, 8);
        insert_sleep_before(&db, end, 7..=29, 4);

        let pace = calculate_pace(&db, &test_config(), end, later())
            .unwrap()
            .expect("pace present");
        assert_close(pace.seven_day_avg, 65.0);
        assert_close(pace.thirty_day_avg, 53.5);
        assert_close(pace.vs_baseline, 11.5);
        assert!(matches!(pace.trend, PaceTrend::Slowing));

        // The week the table shows and the pace's recent window agree.
        let shown = calculate_vitality(
            &db,
            &test_config(),
            &DateRange::last_days(end, std::num::NonZeroU32::new(7).unwrap()),
            later(),
        )
        .unwrap();
        assert_eq!(shown.len(), 7);
        let shown_mean = shown.iter().map(|s| s.total_score).sum::<f64>() / 7.0;
        assert_close(shown_mean, pace.seven_day_avg);
    }

    #[test]
    fn pace_windows_end_at_the_given_date() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");
        insert_sleep_before(&db, end, 0..=6, 8);
        insert_sleep_before(&db, end, [7], 4); // 03-24: in the baseline, not in the recent window
        insert_sleep_before(&db, end, 8..=29, 8);
        insert_sleep_before(&db, end, [30], 4); // 03-01: one date before the baseline
        for day in 1..=7 {
            // April: after `end`, in neither window
            insert_sleep_hours(&db, &format!("2026-04-{day:02}"), 4);
        }

        let pace = calculate_pace(&db, &test_config(), end, later())
            .unwrap()
            .expect("pace present");
        assert_close(pace.seven_day_avg, 65.0);
        assert_close(pace.thirty_day_avg, (29.0 * 65.0 + 50.0) / 30.0);
    }

    #[test]
    fn pace_with_5_of_the_7_recent_days_scored() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");
        insert_sleep_before(&db, end, (0..=4).chain(7..=21), 8);

        let pace = calculate_pace(&db, &test_config(), end, later())
            .unwrap()
            .expect("pace present");
        assert_close(pace.seven_day_avg, 65.0);
    }

    #[test]
    fn no_pace_with_4_of_the_7_recent_days_scored() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");
        insert_sleep_before(&db, end, (0..=3).chain(7..=22), 8);

        assert!(
            calculate_pace(&db, &test_config(), end, later())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn pace_with_20_of_the_30_days_scored() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");
        insert_sleep_before(&db, end, (0..=6).chain(7..=19), 8);

        assert!(
            calculate_pace(&db, &test_config(), end, later())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn no_pace_with_19_of_the_30_days_scored() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");
        insert_sleep_before(&db, end, (0..=6).chain(7..=18), 8);

        assert!(
            calculate_pace(&db, &test_config(), end, later())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn no_pace_without_scores() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");

        assert!(
            calculate_pace(&db, &test_config(), end, later())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn pace_slowing_when_recent_above_baseline() {
        // 7 recent days at 77, 23 older days at 50
        let end = date("2026-03-31");
        let mut totals = repeat_score(77.0, 7);
        totals.extend(repeat_score(50.0, 23));
        let pace = pace_from_scores(&daily_scores(end, &totals), end).unwrap();
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
        // 7 recent days at 50, 23 older days at 80
        let end = date("2026-03-31");
        let mut totals = repeat_score(50.0, 7);
        totals.extend(repeat_score(80.0, 23));
        let pace = pace_from_scores(&daily_scores(end, &totals), end).unwrap();
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
        // All 30 days at 70 → multiplier = 1.0
        let end = date("2026-03-31");
        let pace = pace_from_scores(&daily_scores(end, &repeat_score(70.0, 30)), end).unwrap();
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
        // 7 recent = 75, 23 older = 60 → 7d_avg=75, 30d_avg=(75*7+60*23)/30=63.5
        let end = date("2026-03-31");
        let mut totals = repeat_score(75.0, 7);
        totals.extend(repeat_score(60.0, 23));
        let pace = pace_from_scores(&daily_scores(end, &totals), end).unwrap();
        assert_close(pace.seven_day_avg, 75.0);
        assert_close(pace.thirty_day_avg, 63.5);
        assert_close(pace.vs_baseline, 11.5);
    }

    // -----------------------------------------------------------------------
    // Unfinished days (#26)
    // -----------------------------------------------------------------------

    #[test]
    fn a_day_is_complete_when_it_is_before_today() {
        let today = date("2026-03-31");
        assert!(is_complete(date("2026-03-30"), today));
        assert!(!is_complete(today, today));
        assert!(!is_complete(date("2026-04-01"), today));
    }

    #[test]
    fn vitality_marks_only_the_days_from_today_on_as_unfinished() {
        let db = crate::db::Database::open_memory().unwrap();
        let today = date("2026-03-31");
        insert_sleep_before(&db, today, 0..=2, 8);

        let range = DateRange::last_days(today, std::num::NonZeroU32::new(3).unwrap());
        let scores = calculate_vitality(&db, &test_config(), &range, today).unwrap();

        let marks: Vec<(&str, bool)> = scores
            .iter()
            .map(|s| (s.date.as_str(), s.complete))
            .collect();
        assert_eq!(
            marks,
            [
                ("2026-03-31", false),
                ("2026-03-30", true),
                ("2026-03-29", true)
            ]
        );
    }

    #[test]
    fn pace_ending_today_leaves_today_out() {
        let end = date("2026-03-31");
        let with_today = crate::db::Database::open_memory().unwrap();
        insert_sleep_before(&with_today, end, [0], 4); // today so far: 50.0, unlike the rest
        insert_sleep_before(&with_today, end, 1..=29, 8);
        let without_today = crate::db::Database::open_memory().unwrap();
        insert_sleep_before(&without_today, end, 1..=29, 8);

        let pace = calculate_pace(&with_today, &test_config(), end, end)
            .unwrap()
            .expect("pace present");
        let expected = calculate_pace(&without_today, &test_config(), end, end)
            .unwrap()
            .expect("pace present");

        assert_close(pace.seven_day_avg, 65.0);
        assert_close(pace.thirty_day_avg, 65.0);
        assert_close(pace.seven_day_avg, expected.seven_day_avg);
        assert_close(pace.thirty_day_avg, expected.thirty_day_avg);
        assert_close(pace.vs_baseline, expected.vs_baseline);
    }

    #[test]
    fn unfinished_today_does_not_count_towards_the_minimums() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");
        // Today and 4 more of the 7 recent days: 5 scored, 4 of them finished.
        insert_sleep_before(&db, end, (0..=4).chain(7..=25), 8);

        assert!(
            calculate_pace(&db, &test_config(), end, later())
                .unwrap()
                .is_some(),
            "with today finished there are 5 of 7"
        );
        assert!(
            calculate_pace(&db, &test_config(), end, end)
                .unwrap()
                .is_none(),
            "with today unfinished there are 4 of 7"
        );
    }

    #[test]
    fn unfinished_today_does_not_count_towards_the_30_day_minimum() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");
        // Today and 19 earlier days: 20 scored, 19 of them finished.
        insert_sleep_before(&db, end, (0..=6).chain(7..=19), 8);

        assert!(
            calculate_pace(&db, &test_config(), end, later())
                .unwrap()
                .is_some()
        );
        assert!(
            calculate_pace(&db, &test_config(), end, end)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn pace_ending_before_today_counts_every_day() {
        let db = crate::db::Database::open_memory().unwrap();
        let end = date("2026-03-31");
        insert_sleep_before(&db, end, [0], 4);
        insert_sleep_before(&db, end, 1..=29, 8);

        let pace = calculate_pace(&db, &test_config(), end, date("2026-04-01"))
            .unwrap()
            .expect("pace present");
        assert_close(pace.seven_day_avg, (6.0 * 65.0 + 50.0) / 7.0);
        assert_close(pace.thirty_day_avg, (29.0 * 65.0 + 50.0) / 30.0);
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
        assert!(
            (score - 100.0).abs() < 0.01,
            "Expected 100.0 (cap), got {score}"
        );
    }

    // -----------------------------------------------------------------------
    // calculate_fitness_score / compute_total_score
    // -----------------------------------------------------------------------

    #[test]
    fn fitness_score_at_poor_vo2() {
        // VO2=28.1 → ~19.3; no LBM
        let score = calculate_fitness_score(Some(28.1), None, None);
        assert!(score.is_some());
        assert!(
            (score.unwrap() - 19.3).abs() < 0.2,
            "Expected ~19.3, got {:?}",
            score
        );
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
        assert!(
            (score.unwrap() - 100.0).abs() < 0.01,
            "Expected 100.0, got {:?}",
            score
        );
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
        assert!(
            with_fitness < without,
            "lower fitness should pull total down"
        );
    }
}
