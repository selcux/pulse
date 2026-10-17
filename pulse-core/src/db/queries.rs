//! Database query functions for all health metrics.
//!
//! All functions take a [`Database`] reference. Range queries take a [`DateRange`] (both ends
//! included) and return rows newest-first: `query_sleep(db, &range)` with a range from
//! [`DateRange::last_days`] of 7 returns the 7 dates ending today.

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::Database;
use crate::date_range::DateRange;
use crate::models::{Activity, ExerciseSet, Heart, Recovery, Sleep, Stress, Wellness, Workout};

/// The first and last date of `range` as the `YYYY-MM-DD` text the date columns hold.
fn date_params(range: &DateRange) -> (String, String) {
    (range.start.to_string(), range.end.to_string())
}

// ---------------------------------------------------------------------------
// Sleep
// ---------------------------------------------------------------------------

/// Insert or replace a sleep record for `s.date`.
pub fn upsert_sleep(db: &Database, s: &Sleep) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO sleep (date, total_seconds, deep_seconds, rem_seconds, light_seconds, awake_seconds, sleep_score, hrv_ms, start_time, end_time, spo2_avg, respiration_avg, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            s.date,
            s.total_seconds,
            s.deep_seconds,
            s.rem_seconds,
            s.light_seconds,
            s.awake_seconds,
            s.sleep_score,
            s.hrv_ms,
            s.start_time,
            s.end_time,
            s.spo2_avg,
            s.respiration_avg,
            s.source,
        ],
    )?;
    Ok(())
}

/// Columns of `sleep` in `Sleep` field order; `sleep_row` reads them by position.
const SLEEP_COLUMNS: &str = "date, total_seconds, deep_seconds, rem_seconds, light_seconds, awake_seconds, sleep_score, hrv_ms, start_time, end_time, spo2_avg, respiration_avg, source";

/// Map a row selected with `SLEEP_COLUMNS` to a `Sleep`.
fn sleep_row(row: &rusqlite::Row) -> rusqlite::Result<Sleep> {
    Ok(Sleep {
        date: row.get(0)?,
        total_seconds: row.get(1)?,
        deep_seconds: row.get(2)?,
        rem_seconds: row.get(3)?,
        light_seconds: row.get(4)?,
        awake_seconds: row.get(5)?,
        sleep_score: row.get(6)?,
        hrv_ms: row.get(7)?,
        start_time: row.get(8)?,
        end_time: row.get(9)?,
        spo2_avg: row.get(10)?,
        respiration_avg: row.get(11)?,
        source: row.get(12)?,
    })
}

/// Return sleep records dated within `range` (both ends included), newest-first.
pub fn query_sleep(db: &Database, range: &DateRange) -> Result<Vec<Sleep>> {
    let mut stmt = db.conn().prepare(&format!(
        "SELECT {SLEEP_COLUMNS} FROM sleep WHERE date BETWEEN ?1 AND ?2 ORDER BY date DESC"
    ))?;
    let (start, end) = date_params(range);
    let rows = stmt.query_map(params![start, end], sleep_row)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Return the sleep record for a specific `date` (`YYYY-MM-DD`), or `None`.
pub fn get_sleep(db: &Database, date: &str) -> Result<Option<Sleep>> {
    let mut stmt = db.conn().prepare(&format!(
        "SELECT {SLEEP_COLUMNS} FROM sleep WHERE date = ?1"
    ))?;
    let result = stmt.query_row(params![date], sleep_row).optional()?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Heart
// ---------------------------------------------------------------------------

/// Insert or replace a heart-rate record for `h.date`.
pub fn upsert_heart(db: &Database, h: &Heart) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO heart (date, resting_hr, max_hr, min_hr, hrv_avg, vo2_max, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            h.date,
            h.resting_hr,
            h.max_hr,
            h.min_hr,
            h.hrv_avg,
            h.vo2_max,
            h.source
        ],
    )?;
    Ok(())
}

/// Return heart records dated within `range` (both ends included), newest-first.
pub fn query_heart(db: &Database, range: &DateRange) -> Result<Vec<Heart>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, resting_hr, max_hr, min_hr, hrv_avg, vo2_max, source
         FROM heart WHERE date BETWEEN ?1 AND ?2 ORDER BY date DESC",
    )?;
    let (start, end) = date_params(range);
    let rows = stmt.query_map(params![start, end], |row| {
        Ok(Heart {
            date: row.get(0)?,
            resting_hr: row.get(1)?,
            max_hr: row.get(2)?,
            min_hr: row.get(3)?,
            hrv_avg: row.get(4)?,
            vo2_max: row.get(5)?,
            source: row.get(6)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------------------
// Heart baselines (30-day averages)
// ---------------------------------------------------------------------------

/// Averages over a date range, used as personal baselines for recovery scoring.
pub struct HeartBaselines {
    /// Average resting heart rate (bpm).
    pub rhr_avg: Option<f64>,
    /// Average HRV (ms).
    pub hrv_avg: Option<f64>,
}

/// Average resting HR and HRV over `range`; vitality passes the 30 days ending at the last scored day.
pub fn compute_heart_baselines(db: &Database, range: &DateRange) -> Result<HeartBaselines> {
    let mut stmt = db
        .conn()
        .prepare("SELECT AVG(resting_hr), AVG(hrv_avg) FROM heart WHERE date BETWEEN ?1 AND ?2")?;
    let (start, end) = date_params(range);
    let result = stmt.query_row(params![start, end], |row| {
        Ok(HeartBaselines {
            rhr_avg: row.get(0)?,
            hrv_avg: row.get(1)?,
        })
    })?;
    Ok(result)
}

/// Average VO2 max over `range`, or `None` without an estimate in it.
pub fn compute_vo2_baseline(db: &Database, range: &DateRange) -> Result<Option<f64>> {
    let mut stmt = db.conn().prepare(
        "SELECT AVG(vo2_max) FROM heart WHERE date BETWEEN ?1 AND ?2 AND vo2_max IS NOT NULL",
    )?;
    let (start, end) = date_params(range);
    let result: Option<f64> = stmt.query_row(params![start, end], |row| row.get(0))?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

/// Insert or replace a recovery record for `r.date`.
pub fn upsert_recovery(db: &Database, r: &Recovery) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO recovery (date, body_battery_charged, body_battery_drained, body_battery_peak, body_battery_low, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            r.date,
            r.body_battery_charged,
            r.body_battery_drained,
            r.body_battery_peak,
            r.body_battery_low,
            r.source,
        ],
    )?;
    Ok(())
}

/// Return recovery records dated within `range` (both ends included), newest-first.
pub fn query_recovery(db: &Database, range: &DateRange) -> Result<Vec<Recovery>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, body_battery_charged, body_battery_drained, body_battery_peak, body_battery_low, source
         FROM recovery WHERE date BETWEEN ?1 AND ?2 ORDER BY date DESC",
    )?;
    let (start, end) = date_params(range);
    let rows = stmt.query_map(params![start, end], |row| {
        Ok(Recovery {
            date: row.get(0)?,
            body_battery_charged: row.get(1)?,
            body_battery_drained: row.get(2)?,
            body_battery_peak: row.get(3)?,
            body_battery_low: row.get(4)?,
            source: row.get(5)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------------------
// Activity
// ---------------------------------------------------------------------------

/// Insert or replace an activity record for `a.date`.
pub fn upsert_activity(db: &Database, a: &Activity) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO activity (date, steps, active_minutes, floors, total_kcal, active_kcal, bmr_kcal, distance_meters, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            a.date,
            a.steps,
            a.active_minutes,
            a.floors,
            a.total_kcal,
            a.active_kcal,
            a.bmr_kcal,
            a.distance_meters,
            a.source,
        ],
    )?;
    Ok(())
}

/// Return activity records dated within `range` (both ends included), newest-first.
pub fn query_activity(db: &Database, range: &DateRange) -> Result<Vec<Activity>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, steps, active_minutes, floors, total_kcal, active_kcal, bmr_kcal, distance_meters, source
         FROM activity WHERE date BETWEEN ?1 AND ?2 ORDER BY date DESC",
    )?;
    let (start, end) = date_params(range);
    let rows = stmt.query_map(params![start, end], |row| {
        Ok(Activity {
            date: row.get(0)?,
            steps: row.get(1)?,
            active_minutes: row.get(2)?,
            floors: row.get(3)?,
            total_kcal: row.get(4)?,
            active_kcal: row.get(5)?,
            bmr_kcal: row.get(6)?,
            distance_meters: row.get(7)?,
            source: row.get(8)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------------------
// Stress
// ---------------------------------------------------------------------------

/// Insert or replace a stress record for `s.date`.
pub fn upsert_stress(db: &Database, s: &Stress) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO stress (date, avg_stress, max_stress, rest_seconds, low_seconds, medium_seconds, high_seconds, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            s.date,
            s.avg_stress,
            s.max_stress,
            s.rest_seconds,
            s.low_seconds,
            s.medium_seconds,
            s.high_seconds,
            s.source,
        ],
    )?;
    Ok(())
}

/// Return stress records dated within `range` (both ends included), newest-first.
pub fn query_stress(db: &Database, range: &DateRange) -> Result<Vec<Stress>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, avg_stress, max_stress, rest_seconds, low_seconds, medium_seconds, high_seconds, source
         FROM stress WHERE date BETWEEN ?1 AND ?2 ORDER BY date DESC",
    )?;
    let (start, end) = date_params(range);
    let rows = stmt.query_map(params![start, end], |row| {
        Ok(Stress {
            date: row.get(0)?,
            avg_stress: row.get(1)?,
            max_stress: row.get(2)?,
            rest_seconds: row.get(3)?,
            low_seconds: row.get(4)?,
            medium_seconds: row.get(5)?,
            high_seconds: row.get(6)?,
            source: row.get(7)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------------------
// Wellness
// ---------------------------------------------------------------------------

/// Insert or replace the wellness record for `w.date`.
pub fn upsert_wellness(db: &Database, w: &Wellness) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO wellness (date, weight_kg, body_fat_percent, ctl, atl, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            w.date,
            w.weight_kg,
            w.body_fat_percent,
            w.ctl,
            w.atl,
            w.source,
        ],
    )?;
    Ok(())
}

/// Return wellness records dated within `range` (both ends included), newest-first.
pub fn query_wellness(db: &Database, range: &DateRange) -> Result<Vec<Wellness>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, weight_kg, body_fat_percent, ctl, atl, source
         FROM wellness WHERE date BETWEEN ?1 AND ?2 ORDER BY date DESC",
    )?;
    let (start, end) = date_params(range);
    let rows = stmt.query_map(params![start, end], |row| {
        Ok(Wellness {
            date: row.get(0)?,
            weight_kg: row.get(1)?,
            body_fat_percent: row.get(2)?,
            ctl: row.get(3)?,
            atl: row.get(4)?,
            source: row.get(5)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------------------
// Workouts
// ---------------------------------------------------------------------------

/// `hr_zone_seconds` as stored: JSON array text such as `[491,158,0]`, the same text the v9
/// backfill's `json_extract` writes.
fn zones_to_text(seconds: Option<&[i64]>) -> Result<Option<String>> {
    seconds
        .map(|s| serde_json::to_string(s).context("Failed to encode heart rate zone times"))
        .transpose()
}

/// Stored `hr_zone_seconds` text back as seconds. Text that is not a JSON array of integers
/// reads as `None`, so a damaged cell hides the zones, not the workout.
fn zones_from_text(text: Option<String>) -> Option<Vec<i64>> {
    serde_json::from_str(&text?).ok()
}

/// Insert or replace a workout record for `w.id`.
///
/// `hr_zone_seconds` is stored as JSON array text, such as `[491,158,0]`.
pub fn upsert_workout(db: &Database, w: &Workout) -> Result<()> {
    let zones = zones_to_text(w.hr_zone_seconds.as_deref())?;
    db.conn().execute(
        "INSERT OR REPLACE INTO workouts (id, name, start_time, duration_seconds, activity_type, calories, avg_hr, max_hr, distance_meters, avg_speed_mps, hr_zone_seconds, training_load, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            w.id,
            w.name.as_deref(),
            w.start_time,
            w.duration_seconds,
            w.activity_type,
            w.calories,
            w.avg_hr,
            w.max_hr,
            w.distance_meters,
            w.avg_speed_mps,
            zones,
            w.training_load,
            w.source,
        ],
    )?;
    Ok(())
}

/// Return workouts whose start date (the first 10 characters of `start_time`) is within `range`,
/// newest-first.
pub fn query_workouts(db: &Database, range: &DateRange) -> Result<Vec<Workout>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, name, start_time, duration_seconds, activity_type, calories, avg_hr, max_hr, distance_meters, avg_speed_mps, hr_zone_seconds, training_load, source
         FROM workouts WHERE substr(start_time, 1, 10) BETWEEN ?1 AND ?2 ORDER BY start_time DESC",
    )?;
    let (start, end) = date_params(range);
    let rows = stmt.query_map(params![start, end], |row| {
        Ok(Workout {
            id: row.get(0)?,
            name: row.get(1)?,
            start_time: row.get(2)?,
            duration_seconds: row.get(3)?,
            activity_type: row.get(4)?,
            calories: row.get(5)?,
            avg_hr: row.get(6)?,
            max_hr: row.get(7)?,
            distance_meters: row.get(8)?,
            avg_speed_mps: row.get(9)?,
            hr_zone_seconds: zones_from_text(row.get(10)?),
            training_load: row.get(11)?,
            source: row.get(12)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------------------
// Exercise Sets
// ---------------------------------------------------------------------------

/// Insert a new exercise set and return the assigned row ID.
pub fn insert_exercise_set(db: &Database, s: &ExerciseSet) -> Result<i64> {
    db.conn().execute(
        "INSERT INTO exercise_sets (workout_id, set_order, exercise_category, exercise_name, repetitions, weight_kg)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            s.workout_id,
            s.set_order,
            s.exercise_category,
            s.exercise_name,
            s.repetitions,
            s.weight_kg,
        ],
    )?;
    Ok(db.conn().last_insert_rowid())
}

/// Return all exercise sets for a workout, ordered by `set_order` ascending.
pub fn query_exercise_sets(db: &Database, workout_id: &str) -> Result<Vec<ExerciseSet>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, workout_id, set_order, exercise_category, exercise_name, repetitions, weight_kg
         FROM exercise_sets WHERE workout_id = ?1 ORDER BY set_order ASC",
    )?;
    let rows = stmt.query_map(params![workout_id], |row| {
        Ok(ExerciseSet {
            id: row.get(0)?,
            workout_id: row.get(1)?,
            set_order: row.get(2)?,
            exercise_category: row.get(3)?,
            exercise_name: row.get(4)?,
            repetitions: row.get(5)?,
            weight_kg: row.get(6)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Delete all exercise sets belonging to `workout_id`.
pub fn delete_exercise_sets_for_workout(db: &Database, workout_id: &str) -> Result<()> {
    db.conn().execute(
        "DELETE FROM exercise_sets WHERE workout_id = ?1",
        params![workout_id],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Raw responses
// ---------------------------------------------------------------------------

/// Insert or replace the stored response body for `(provider, endpoint, key)`.
/// `fetched_at` is set to the current UTC time. `body` must be valid JSON (the table checks it).
pub fn upsert_raw_response(
    db: &Database,
    provider: &str,
    endpoint: &str,
    key: &str,
    body: &str,
) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO raw_responses (provider, endpoint, key, body) VALUES (?1, ?2, ?3, ?4)",
        params![provider, endpoint, key, body],
    )?;
    Ok(())
}

/// Return the stored response body for `(provider, endpoint, key)`, or `None`.
pub fn get_raw_response(
    db: &Database,
    provider: &str,
    endpoint: &str,
    key: &str,
) -> Result<Option<String>> {
    let mut stmt = db.conn().prepare(
        "SELECT body FROM raw_responses WHERE provider = ?1 AND endpoint = ?2 AND key = ?3",
    )?;
    let body = stmt
        .query_row(params![provider, endpoint, key], |row| row.get(0))
        .optional()?;
    Ok(body)
}

/// Delete the stored response body for `(provider, endpoint, key)`, if there is one.
pub fn delete_raw_response(db: &Database, provider: &str, endpoint: &str, key: &str) -> Result<()> {
    db.conn().execute(
        "DELETE FROM raw_responses WHERE provider = ?1 AND endpoint = ?2 AND key = ?3",
        params![provider, endpoint, key],
    )?;
    Ok(())
}

/// Return the keys stored for `provider` and `endpoint`, in ascending order.
pub fn raw_response_keys(db: &Database, provider: &str, endpoint: &str) -> Result<Vec<String>> {
    let mut stmt = db.conn().prepare(
        "SELECT key FROM raw_responses WHERE provider = ?1 AND endpoint = ?2 ORDER BY key",
    )?;
    // Row errors propagate: a dropped key would be a day silently left out of a rebuild.
    let keys = stmt
        .query_map(params![provider, endpoint], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(keys)
}

/// Return the number of stored response bodies, across all providers.
pub fn count_raw_responses(db: &Database) -> Result<i64> {
    let count = db
        .conn()
        .query_row("SELECT COUNT(*) FROM raw_responses", [], |row| row.get(0))?;
    Ok(count)
}

// ---------------------------------------------------------------------------
// Sync State
// ---------------------------------------------------------------------------

/// Retrieve a named sync-state value, or `None` if the key does not exist.
pub fn get_sync_state(db: &Database, key: &str) -> Result<Option<String>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT value FROM sync_state WHERE key = ?1")?;
    let result = stmt
        .query_row(params![key], |row| row.get::<_, Option<String>>(0))
        .optional()?;
    Ok(result.flatten())
}

/// Persist a named sync-state value (insert or replace).
pub fn set_sync_state(db: &Database, key: &str, value: &str) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO sync_state (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params![key, value],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_memory().expect("Failed to open in-memory database")
    }

    #[test]
    fn migrations_run_without_error() {
        let _db = test_db();
        // If we get here, migrations succeeded.
    }

    #[test]
    fn raw_response_upsert_and_get() {
        let db = test_db();
        upsert_raw_response(&db, "garmin", "sleep", "2026-03-01", r#"{"a":1}"#).unwrap();
        assert_eq!(
            get_raw_response(&db, "garmin", "sleep", "2026-03-01")
                .unwrap()
                .as_deref(),
            Some(r#"{"a":1}"#)
        );
        assert!(
            get_raw_response(&db, "garmin", "hrv", "2026-03-01")
                .unwrap()
                .is_none()
        );
        assert!(
            get_raw_response(&db, "garmin", "sleep", "2026-03-02")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn raw_response_upsert_replaces() {
        let db = test_db();
        upsert_raw_response(&db, "garmin", "sleep", "2026-03-01", r#"{"a":1}"#).unwrap();
        upsert_raw_response(&db, "garmin", "sleep", "2026-03-01", r#"{"a":2}"#).unwrap();
        assert_eq!(
            get_raw_response(&db, "garmin", "sleep", "2026-03-01")
                .unwrap()
                .as_deref(),
            Some(r#"{"a":2}"#)
        );
        assert_eq!(count_raw_responses(&db).unwrap(), 1);
    }

    #[test]
    fn raw_response_delete() {
        let db = test_db();
        upsert_raw_response(&db, "garmin", "sleep", "2026-03-01", "{}").unwrap();
        delete_raw_response(&db, "garmin", "sleep", "2026-03-01").unwrap();
        assert!(
            get_raw_response(&db, "garmin", "sleep", "2026-03-01")
                .unwrap()
                .is_none()
        );
        delete_raw_response(&db, "garmin", "sleep", "2026-03-01").unwrap();
    }

    #[test]
    fn raw_response_keys_are_scoped_and_sorted() {
        let db = test_db();
        upsert_raw_response(&db, "garmin", "sleep", "2026-03-02", "{}").unwrap();
        upsert_raw_response(&db, "garmin", "sleep", "2026-03-01", "{}").unwrap();
        upsert_raw_response(&db, "garmin", "hrv", "2026-03-05", "{}").unwrap();
        upsert_raw_response(&db, "intervals", "activity", "i1", "{}").unwrap();
        assert_eq!(
            raw_response_keys(&db, "garmin", "sleep").unwrap(),
            vec!["2026-03-01".to_string(), "2026-03-02".to_string()]
        );
    }

    #[test]
    fn migrations_are_idempotent() {
        let db = test_db();
        // Running migrations again on the same connection should be fine.
        crate::db::migrations::run(&db).expect("Re-running migrations should succeed");
    }

    #[test]
    fn sleep_upsert_and_get() {
        let db = test_db();
        let sleep = Sleep {
            date: "2026-03-01".into(),
            total_seconds: 28800,
            deep_seconds: 7200,
            rem_seconds: 5400,
            light_seconds: 14400,
            awake_seconds: 1800,
            sleep_score: Some(82),
            hrv_ms: Some(45.5),
            start_time: Some("2026-02-28 23:10:00".into()),
            end_time: Some("2026-03-01 06:55:00".into()),
            spo2_avg: Some(95.0),
            respiration_avg: Some(14.5),
            source: "garmin".into(),
        };

        upsert_sleep(&db, &sleep).unwrap();
        let fetched = get_sleep(&db, "2026-03-01").unwrap().expect("should exist");
        assert_eq!(fetched.date, "2026-03-01");
        assert_eq!(fetched.total_seconds, 28800);
        assert_eq!(fetched.sleep_score, Some(82));
        assert!((fetched.hrv_ms.unwrap() - 45.5).abs() < f64::EPSILON);
        assert_eq!(fetched.start_time.as_deref(), Some("2026-02-28 23:10:00"));
        assert_eq!(fetched.end_time.as_deref(), Some("2026-03-01 06:55:00"));
        assert_eq!(fetched.spo2_avg, Some(95.0));
        assert_eq!(fetched.respiration_avg, Some(14.5));
    }

    #[test]
    fn sleep_upsert_replaces() {
        let db = test_db();
        let mut sleep = Sleep {
            date: "2026-03-01".into(),
            total_seconds: 28800,
            deep_seconds: 7200,
            rem_seconds: 5400,
            light_seconds: 14400,
            awake_seconds: 1800,
            sleep_score: Some(70),
            hrv_ms: None,
            start_time: None,
            end_time: None,
            spo2_avg: None,
            respiration_avg: None,
            source: "garmin".into(),
        };

        upsert_sleep(&db, &sleep).unwrap();
        sleep.sleep_score = Some(85);
        upsert_sleep(&db, &sleep).unwrap();

        let fetched = get_sleep(&db, "2026-03-01").unwrap().unwrap();
        assert_eq!(fetched.sleep_score, Some(85));
    }

    #[test]
    fn workout_and_exercise_sets_round_trip() {
        let db = test_db();
        let workout = Workout {
            id: "w-001".into(),
            name: None,
            start_time: "2026-03-01T08:00:00".into(),
            duration_seconds: 3600,
            activity_type: "strength".into(),
            calories: Some(350),
            avg_hr: Some(130),
            max_hr: Some(165),
            distance_meters: None,
            avg_speed_mps: None,
            hr_zone_seconds: None,
            training_load: None,
            source: "intervals".into(),
        };
        upsert_workout(&db, &workout).unwrap();

        let set1 = ExerciseSet {
            id: None,
            workout_id: "w-001".into(),
            set_order: 1,
            exercise_category: Some("chest".into()),
            exercise_name: Some("bench press".into()),
            repetitions: Some(10),
            weight_kg: Some(80.0),
        };
        let set2 = ExerciseSet {
            id: None,
            workout_id: "w-001".into(),
            set_order: 2,
            exercise_category: Some("chest".into()),
            exercise_name: Some("bench press".into()),
            repetitions: Some(8),
            weight_kg: Some(85.0),
        };

        let id1 = insert_exercise_set(&db, &set1).unwrap();
        let id2 = insert_exercise_set(&db, &set2).unwrap();
        assert!(id1 > 0);
        assert!(id2 > id1);

        let sets = query_exercise_sets(&db, "w-001").unwrap();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].set_order, 1);
        assert_eq!(sets[1].set_order, 2);
        assert!((sets[1].weight_kg.unwrap() - 85.0).abs() < f64::EPSILON);
        assert_eq!(sets[0].exercise_name.as_deref(), Some("bench press"));

        // Delete and verify
        delete_exercise_sets_for_workout(&db, "w-001").unwrap();
        let sets = query_exercise_sets(&db, "w-001").unwrap();
        assert!(sets.is_empty());
    }

    #[test]
    fn exercise_set_without_name_round_trips() {
        let db = test_db();
        let workout = Workout {
            id: "w-002".into(),
            name: None,
            start_time: "2026-03-01T08:00:00".into(),
            duration_seconds: 3600,
            activity_type: "strength".into(),
            calories: Some(350),
            avg_hr: Some(130),
            max_hr: Some(165),
            distance_meters: None,
            avg_speed_mps: None,
            hr_zone_seconds: None,
            training_load: None,
            source: "intervals".into(),
        };
        upsert_workout(&db, &workout).unwrap();

        let set = ExerciseSet {
            id: None,
            workout_id: "w-002".into(),
            set_order: 1,
            exercise_category: None,
            exercise_name: None,
            repetitions: Some(12),
            weight_kg: None,
        };
        insert_exercise_set(&db, &set).unwrap();

        // query_exercise_sets drops rows it cannot map, so the length matters here.
        let sets = query_exercise_sets(&db, "w-002").unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].exercise_name, None);
        assert_eq!(sets[0].exercise_category, None);
        assert_eq!(sets[0].repetitions, Some(12));
    }

    #[test]
    fn sync_state_get_set() {
        let db = test_db();

        // Non-existent key returns None.
        assert!(get_sync_state(&db, "last_garmin_sync").unwrap().is_none());

        set_sync_state(&db, "last_garmin_sync", "2026-03-01").unwrap();
        let val = get_sync_state(&db, "last_garmin_sync")
            .unwrap()
            .expect("should exist");
        assert_eq!(val, "2026-03-01");

        // Overwrite
        set_sync_state(&db, "last_garmin_sync", "2026-03-02").unwrap();
        let val = get_sync_state(&db, "last_garmin_sync").unwrap().unwrap();
        assert_eq!(val, "2026-03-02");
    }

    fn date(s: &str) -> chrono::NaiveDate {
        s.parse().unwrap()
    }

    fn range(start: &str, end: &str) -> DateRange {
        DateRange {
            start: date(start),
            end: date(end),
        }
    }

    fn sleep_on(date: &str) -> Sleep {
        Sleep {
            date: date.into(),
            total_seconds: 25000,
            deep_seconds: 6000,
            rem_seconds: 5000,
            light_seconds: 12000,
            awake_seconds: 2000,
            sleep_score: None,
            hrv_ms: None,
            start_time: None,
            end_time: None,
            spo2_avg: None,
            respiration_avg: None,
            source: "garmin".into(),
        }
    }

    fn heart_on(date: &str) -> Heart {
        Heart {
            date: date.into(),
            resting_hr: None,
            max_hr: None,
            min_hr: None,
            hrv_avg: None,
            vo2_max: None,
            source: "garmin".into(),
        }
    }

    fn recovery_on(date: &str) -> Recovery {
        Recovery {
            date: date.into(),
            body_battery_charged: None,
            body_battery_drained: None,
            body_battery_peak: None,
            body_battery_low: None,
            source: "garmin".into(),
        }
    }

    fn activity_on(date: &str) -> Activity {
        Activity {
            date: date.into(),
            steps: None,
            active_minutes: None,
            floors: None,
            total_kcal: None,
            active_kcal: None,
            bmr_kcal: None,
            distance_meters: None,
            source: "garmin".into(),
        }
    }

    fn stress_on(date: &str) -> Stress {
        Stress {
            date: date.into(),
            avg_stress: None,
            max_stress: None,
            rest_seconds: None,
            low_seconds: None,
            medium_seconds: None,
            high_seconds: None,
            source: "garmin".into(),
        }
    }

    fn wellness_on(date: &str) -> Wellness {
        Wellness {
            date: date.into(),
            weight_kg: None,
            body_fat_percent: None,
            ctl: None,
            atl: None,
            source: "intervals".into(),
        }
    }

    fn workout_at(id: &str, start_time: &str) -> Workout {
        Workout {
            id: id.into(),
            name: None,
            start_time: start_time.into(),
            duration_seconds: 3600,
            activity_type: "strength".into(),
            calories: None,
            avg_hr: None,
            max_hr: None,
            distance_meters: None,
            avg_speed_mps: None,
            hr_zone_seconds: None,
            training_load: None,
            source: "intervals".into(),
        }
    }

    #[test]
    fn workout_round_trips_hr_zone_seconds() {
        let db = test_db();
        let walk = Workout {
            hr_zone_seconds: Some(vec![491, 158, 150, 110, 28, 0, 0]),
            ..workout_at("w-walk", "2026-10-15T07:00:00")
        };
        let run = Workout {
            hr_zone_seconds: Some(vec![0, 0, 600, 1200, 30]),
            ..workout_at("w-run", "2026-10-14T07:00:00")
        };
        upsert_workout(&db, &walk).unwrap();
        upsert_workout(&db, &run).unwrap();
        upsert_workout(&db, &workout_at("w-lift", "2026-10-13T18:00:00")).unwrap();

        let got = query_workouts(&db, &range("2026-10-13", "2026-10-15")).unwrap();

        assert_eq!(got.len(), 3);
        assert_eq!(got[0].id, "w-walk");
        assert_eq!(
            got[0].hr_zone_seconds,
            Some(vec![491, 158, 150, 110, 28, 0, 0])
        );
        assert_eq!(got[1].id, "w-run");
        assert_eq!(got[1].hr_zone_seconds, Some(vec![0, 0, 600, 1200, 30]));
        assert_eq!(got[2].id, "w-lift");
        assert_eq!(got[2].hr_zone_seconds, None);

        // The stored text is what the v9 migration's backfill also writes.
        let stored: Option<String> = db
            .conn()
            .query_row(
                "SELECT hr_zone_seconds FROM workouts WHERE id = 'w-walk'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored.as_deref(), Some("[491,158,150,110,28,0,0]"));
    }

    #[test]
    fn query_workouts_reads_an_undecodable_zone_value_as_none() {
        let db = test_db();
        let walk = Workout {
            hr_zone_seconds: Some(vec![491, 158, 0]),
            ..workout_at("w-walk", "2026-10-15T07:00:00")
        };
        upsert_workout(&db, &walk).unwrap();
        db.conn()
            .execute(
                "UPDATE workouts SET hr_zone_seconds = 'not json' WHERE id = 'w-walk'",
                [],
            )
            .unwrap();

        let got = query_workouts(&db, &range("2026-10-15", "2026-10-15")).unwrap();

        assert_eq!(got.len(), 1);
        assert_eq!(got[0].hr_zone_seconds, None);
    }

    #[test]
    fn workout_round_trips_training_load() {
        let db = test_db();
        let walk = Workout {
            training_load: Some(43),
            ..workout_at("w-walk", "2026-10-15T07:00:00")
        };
        upsert_workout(&db, &walk).unwrap();
        upsert_workout(&db, &workout_at("w-lift", "2026-10-14T18:00:00")).unwrap();

        let got = query_workouts(&db, &range("2026-10-14", "2026-10-15")).unwrap();

        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "w-walk");
        assert_eq!(got[0].training_load, Some(43));
        assert_eq!(got[1].id, "w-lift");
        assert_eq!(got[1].training_load, None);
    }

    #[test]
    fn workout_round_trips_distance_and_speed() {
        let db = test_db();
        let walk = Workout {
            distance_meters: Some(5043.21),
            avg_speed_mps: Some(1.417),
            ..workout_at("w-walk", "2026-10-15T07:00:00")
        };
        upsert_workout(&db, &walk).unwrap();
        upsert_workout(&db, &workout_at("w-lift", "2026-10-14T18:00:00")).unwrap();

        let got = query_workouts(&db, &range("2026-10-14", "2026-10-15")).unwrap();

        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "w-walk");
        assert_eq!(got[0].distance_meters, Some(5043.21));
        assert_eq!(got[0].avg_speed_mps, Some(1.417));
        assert_eq!(got[1].id, "w-lift");
        assert_eq!(got[1].distance_meters, None);
        assert_eq!(got[1].avg_speed_mps, None);
    }

    #[test]
    fn daily_queries_include_both_ends_of_the_range_newest_first() {
        let db = test_db();
        let days = [
            "2026-02-28",
            "2026-03-01",
            "2026-03-04",
            "2026-03-07",
            "2026-03-08",
        ];
        for d in days {
            upsert_sleep(&db, &sleep_on(d)).unwrap();
            upsert_heart(&db, &heart_on(d)).unwrap();
            upsert_recovery(&db, &recovery_on(d)).unwrap();
            upsert_activity(&db, &activity_on(d)).unwrap();
            upsert_stress(&db, &stress_on(d)).unwrap();
            upsert_wellness(&db, &wellness_on(d)).unwrap();
        }
        let march_week = range("2026-03-01", "2026-03-07");
        let expected = ["2026-03-07", "2026-03-04", "2026-03-01"];

        let got: Vec<String> = query_sleep(&db, &march_week)
            .unwrap()
            .into_iter()
            .map(|r| r.date)
            .collect();
        assert_eq!(got, expected, "sleep");
        let got: Vec<String> = query_heart(&db, &march_week)
            .unwrap()
            .into_iter()
            .map(|r| r.date)
            .collect();
        assert_eq!(got, expected, "heart");
        let got: Vec<String> = query_recovery(&db, &march_week)
            .unwrap()
            .into_iter()
            .map(|r| r.date)
            .collect();
        assert_eq!(got, expected, "recovery");
        let got: Vec<String> = query_activity(&db, &march_week)
            .unwrap()
            .into_iter()
            .map(|r| r.date)
            .collect();
        assert_eq!(got, expected, "activity");
        let got: Vec<String> = query_stress(&db, &march_week)
            .unwrap()
            .into_iter()
            .map(|r| r.date)
            .collect();
        assert_eq!(got, expected, "stress");
        let got: Vec<String> = query_wellness(&db, &march_week)
            .unwrap()
            .into_iter()
            .map(|r| r.date)
            .collect();
        assert_eq!(got, expected, "wellness");
    }

    #[test]
    fn query_workouts_matches_the_start_date_over_the_whole_last_day() {
        let db = test_db();
        for (id, start) in [
            ("w0", "2026-02-28T23:59:59"),
            ("w1", "2026-03-01T00:00:00"),
            ("w2", "2026-03-07T23:59:59"),
            ("w3", "2026-03-08T00:00:00"),
        ] {
            upsert_workout(&db, &workout_at(id, start)).unwrap();
        }

        let got: Vec<String> = query_workouts(&db, &range("2026-03-01", "2026-03-07"))
            .unwrap()
            .into_iter()
            .map(|w| w.id)
            .collect();
        assert_eq!(got, ["w2", "w1"]);
    }

    #[test]
    fn range_queries_with_start_after_end_return_nothing() {
        let db = test_db();
        upsert_sleep(&db, &sleep_on("2026-03-04")).unwrap();
        assert!(
            query_sleep(&db, &range("2026-03-07", "2026-03-01"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn heart_baselines_average_only_the_range() {
        let db = test_db();
        for (d, rhr, hrv) in [
            ("2026-02-28", 40, 110.0),
            ("2026-03-01", 60, 50.0),
            ("2026-03-31", 62, 54.0),
            ("2026-04-01", 40, 110.0),
        ] {
            upsert_heart(
                &db,
                &Heart {
                    resting_hr: Some(rhr),
                    hrv_avg: Some(hrv),
                    ..heart_on(d)
                },
            )
            .unwrap();
        }

        let baselines = compute_heart_baselines(&db, &range("2026-03-01", "2026-03-31")).unwrap();
        assert!((baselines.rhr_avg.unwrap() - 61.0).abs() < 0.01);
        assert!((baselines.hrv_avg.unwrap() - 52.0).abs() < 0.01);
    }

    #[test]
    fn vo2_baseline_ignores_estimates_outside_the_range() {
        let db = test_db();
        for (d, vo2) in [
            ("2026-02-28", Some(30.0)),
            ("2026-03-01", Some(50.0)),
            ("2026-03-15", None),
            ("2026-03-31", Some(46.0)),
            ("2026-04-01", Some(30.0)),
        ] {
            upsert_heart(
                &db,
                &Heart {
                    vo2_max: vo2,
                    ..heart_on(d)
                },
            )
            .unwrap();
        }

        let baseline = compute_vo2_baseline(&db, &range("2026-03-01", "2026-03-31")).unwrap();
        assert!((baseline.unwrap() - 48.0).abs() < 0.01);
    }

    #[test]
    fn query_sleep_excludes_dates_outside_the_range() {
        let db = test_db();

        // Insert data with a date far outside the range: it should not appear.
        let old = Sleep {
            date: "2020-01-01".into(),
            total_seconds: 25000,
            deep_seconds: 6000,
            rem_seconds: 5000,
            light_seconds: 12000,
            awake_seconds: 2000,
            sleep_score: None,
            hrv_ms: None,
            start_time: None,
            end_time: None,
            spo2_avg: None,
            respiration_avg: None,
            source: "garmin".into(),
        };
        upsert_sleep(&db, &old).unwrap();

        let results = query_sleep(&db, &range("2026-03-01", "2026-03-07")).unwrap();
        // The old record should be excluded by the date filter.
        assert!(results.is_empty());
    }

    #[test]
    fn get_sleep_returns_none_for_missing_date() {
        let db = test_db();
        assert!(get_sleep(&db, "9999-12-31").unwrap().is_none());
    }

    #[test]
    fn compute_vo2_baseline_returns_avg() {
        let db = Database::open_memory().unwrap();
        upsert_heart(
            &db,
            &Heart {
                date: "2026-03-01".into(),
                resting_hr: None,
                max_hr: None,
                min_hr: None,
                hrv_avg: None,
                vo2_max: Some(50.0),
                source: "garmin".into(),
            },
        )
        .unwrap();
        let baseline = compute_vo2_baseline(&db, &range("2026-03-01", "2026-03-01")).unwrap();
        assert!((baseline.unwrap() - 50.0).abs() < 0.01);
    }

    #[test]
    fn compute_vo2_baseline_returns_none_when_no_data() {
        let db = Database::open_memory().unwrap();
        let baseline = compute_vo2_baseline(&db, &range("2026-03-01", "2026-03-31")).unwrap();
        assert!(baseline.is_none());
    }

    #[test]
    fn heart_upsert_and_query_with_vo2_max() {
        let db = Database::open_memory().unwrap();
        upsert_heart(
            &db,
            &Heart {
                date: "2026-03-01".into(),
                resting_hr: Some(58),
                max_hr: None,
                min_hr: None,
                hrv_avg: Some(50.0),
                vo2_max: Some(48.5),
                source: "garmin".into(),
            },
        )
        .unwrap();
        let results = query_heart(&db, &range("2026-03-01", "2026-03-01")).unwrap();
        assert_eq!(results.len(), 1);
        assert!((results[0].vo2_max.unwrap() - 48.5).abs() < 0.01);
    }

    #[test]
    fn activity_upsert_and_query_round_trips_calories_and_distance() {
        let db = Database::open_memory().unwrap();
        upsert_activity(
            &db,
            &Activity {
                date: "2026-03-01".into(),
                steps: Some(12345),
                active_minutes: Some(60),
                floors: Some(10),
                total_kcal: Some(2450.0),
                active_kcal: Some(612.5),
                bmr_kcal: Some(1838.0),
                distance_meters: Some(8420.0),
                source: "garmin".into(),
            },
        )
        .unwrap();
        let results = query_activity(&db, &range("2026-03-01", "2026-03-01")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].total_kcal, Some(2450.0));
        assert_eq!(results[0].active_kcal, Some(612.5));
        assert_eq!(results[0].bmr_kcal, Some(1838.0));
        assert_eq!(results[0].distance_meters, Some(8420.0));
    }

    #[test]
    fn stress_upsert_and_query_round_trips_durations() {
        let db = Database::open_memory().unwrap();
        upsert_stress(
            &db,
            &Stress {
                date: "2026-03-01".into(),
                avg_stress: Some(35),
                max_stress: Some(78),
                rest_seconds: Some(28800),
                low_seconds: Some(18000),
                medium_seconds: Some(4200),
                high_seconds: None,
                source: "garmin".into(),
            },
        )
        .unwrap();
        let results = query_stress(&db, &range("2026-03-01", "2026-03-01")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rest_seconds, Some(28800));
        assert_eq!(results[0].low_seconds, Some(18000));
        assert_eq!(results[0].medium_seconds, Some(4200));
        assert_eq!(results[0].high_seconds, None);
    }

    #[test]
    fn wellness_upsert_and_query_round_trips_every_field() {
        let db = Database::open_memory().unwrap();
        let row = Wellness {
            date: "2026-03-01".into(),
            weight_kg: Some(78.4),
            body_fat_percent: Some(18.2),
            ctl: Some(52.34),
            atl: Some(0.0),
            source: "intervals".into(),
        };
        upsert_wellness(&db, &row).unwrap();
        let results = query_wellness(&db, &range("2026-03-01", "2026-03-01")).unwrap();
        assert_eq!(results, vec![row]);
        assert_eq!(results[0].atl, Some(0.0), "0 is a value, not NULL");
    }

    #[test]
    fn wellness_upsert_replaces_the_whole_day() {
        let db = Database::open_memory().unwrap();
        upsert_wellness(
            &db,
            &Wellness {
                weight_kg: Some(78.4),
                ..wellness_on("2026-03-01")
            },
        )
        .unwrap();
        upsert_wellness(
            &db,
            &Wellness {
                weight_kg: None,
                ctl: Some(50.0),
                ..wellness_on("2026-03-01")
            },
        )
        .unwrap();
        let results = query_wellness(&db, &range("2026-03-01", "2026-03-01")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].weight_kg, None, "a deleted weigh-in is cleared");
        assert_eq!(results[0].ctl, Some(50.0));
    }
}
