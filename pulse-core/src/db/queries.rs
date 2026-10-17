//! Database query functions for all health metrics.
//!
//! All functions take a [`Database`] reference. Date-range queries use a
//! `days` parameter: `query_sleep(db, 7)` returns records from the last 7 days
//! ordered newest-first.

use anyhow::Result;
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::Database;
use crate::models::{Activity, ExerciseSet, Heart, Recovery, Sleep, Stress, Workout};

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

/// Return sleep records from the last `days` days, newest-first.
pub fn query_sleep(db: &Database, days: i32) -> Result<Vec<Sleep>> {
    let mut stmt = db.conn().prepare(&format!(
        "SELECT {SLEEP_COLUMNS} FROM sleep WHERE date >= date('now', ?1) ORDER BY date DESC"
    ))?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map(params![modifier], sleep_row)?;
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

/// Return heart records from the last `days` days, newest-first.
pub fn query_heart(db: &Database, days: i32) -> Result<Vec<Heart>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, resting_hr, max_hr, min_hr, hrv_avg, vo2_max, source
         FROM heart WHERE date >= date('now', ?1) ORDER BY date DESC",
    )?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map(params![modifier], |row| {
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

/// 30-day rolling averages used as personal baselines for recovery scoring.
pub struct HeartBaselines {
    /// 30-day average resting heart rate (bpm).
    pub rhr_avg: Option<f64>,
    /// 30-day average HRV (ms).
    pub hrv_avg: Option<f64>,
}

/// Compute 30-day rolling averages for resting HR and HRV from stored data.
pub fn compute_heart_baselines(db: &Database) -> Result<HeartBaselines> {
    let mut stmt = db.conn().prepare(
        "SELECT AVG(resting_hr), AVG(hrv_avg) FROM heart WHERE date >= date('now', '-30 days')",
    )?;
    let result = stmt.query_row([], |row| {
        Ok(HeartBaselines {
            rhr_avg: row.get(0)?,
            hrv_avg: row.get(1)?,
        })
    })?;
    Ok(result)
}

/// Compute the 30-day average VO2 Max from stored heart data, or `None` if unavailable.
pub fn compute_vo2_baseline(db: &Database) -> Result<Option<f64>> {
    let mut stmt = db.conn().prepare(
        "SELECT AVG(vo2_max) FROM heart WHERE date >= date('now', '-30 days') AND vo2_max IS NOT NULL",
    )?;
    let result: Option<f64> = stmt.query_row([], |row| row.get(0))?;
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

/// Return recovery records from the last `days` days, newest-first.
pub fn query_recovery(db: &Database, days: i32) -> Result<Vec<Recovery>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, body_battery_charged, body_battery_drained, body_battery_peak, body_battery_low, source
         FROM recovery WHERE date >= date('now', ?1) ORDER BY date DESC",
    )?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map(params![modifier], |row| {
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

/// Return activity records from the last `days` days, newest-first.
pub fn query_activity(db: &Database, days: i32) -> Result<Vec<Activity>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, steps, active_minutes, floors, total_kcal, active_kcal, bmr_kcal, distance_meters, source
         FROM activity WHERE date >= date('now', ?1) ORDER BY date DESC",
    )?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map(params![modifier], |row| {
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

/// Return stress records from the last `days` days, newest-first.
pub fn query_stress(db: &Database, days: i32) -> Result<Vec<Stress>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, avg_stress, max_stress, rest_seconds, low_seconds, medium_seconds, high_seconds, source
         FROM stress WHERE date >= date('now', ?1) ORDER BY date DESC",
    )?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map(params![modifier], |row| {
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
// Workouts
// ---------------------------------------------------------------------------

/// Insert or replace a workout record for `w.id`.
pub fn upsert_workout(db: &Database, w: &Workout) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO workouts (id, name, start_time, duration_seconds, activity_type, calories, avg_hr, max_hr, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            w.id,
            w.name.as_deref(),
            w.start_time,
            w.duration_seconds,
            w.activity_type,
            w.calories,
            w.avg_hr,
            w.max_hr,
            w.source,
        ],
    )?;
    Ok(())
}

/// Return workouts with `start_time` in the last `days` days, newest-first.
pub fn query_workouts(db: &Database, days: i32) -> Result<Vec<Workout>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, name, start_time, duration_seconds, activity_type, calories, avg_hr, max_hr, source
         FROM workouts WHERE start_time >= datetime('now', ?1) ORDER BY start_time DESC",
    )?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map(params![modifier], |row| {
        Ok(Workout {
            id: row.get(0)?,
            name: row.get(1)?,
            start_time: row.get(2)?,
            duration_seconds: row.get(3)?,
            activity_type: row.get(4)?,
            calories: row.get(5)?,
            avg_hr: row.get(6)?,
            max_hr: row.get(7)?,
            source: row.get(8)?,
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

    #[test]
    fn query_sleep_with_days_filter() {
        let db = test_db();

        // Insert data with a date far in the past — should not appear in a 7-day query.
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

        let results = query_sleep(&db, 7).unwrap();
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
        let today = chrono::Local::now().date_naive().to_string();
        upsert_heart(
            &db,
            &Heart {
                date: today.clone(),
                resting_hr: None,
                max_hr: None,
                min_hr: None,
                hrv_avg: None,
                vo2_max: Some(50.0),
                source: "garmin".into(),
            },
        )
        .unwrap();
        let baseline = compute_vo2_baseline(&db).unwrap();
        assert!((baseline.unwrap() - 50.0).abs() < 0.01);
    }

    #[test]
    fn compute_vo2_baseline_returns_none_when_no_data() {
        let db = Database::open_memory().unwrap();
        let baseline = compute_vo2_baseline(&db).unwrap();
        assert!(baseline.is_none());
    }

    #[test]
    fn heart_upsert_and_query_with_vo2_max() {
        let db = Database::open_memory().unwrap();
        let today = chrono::Local::now().date_naive().to_string();
        upsert_heart(
            &db,
            &Heart {
                date: today.clone(),
                resting_hr: Some(58),
                max_hr: None,
                min_hr: None,
                hrv_avg: Some(50.0),
                vo2_max: Some(48.5),
                source: "garmin".into(),
            },
        )
        .unwrap();
        let results = query_heart(&db, 7).unwrap();
        assert_eq!(results.len(), 1);
        assert!((results[0].vo2_max.unwrap() - 48.5).abs() < 0.01);
    }

    #[test]
    fn activity_upsert_and_query_round_trips_calories_and_distance() {
        let db = Database::open_memory().unwrap();
        let today = chrono::Local::now().date_naive().to_string();
        upsert_activity(
            &db,
            &Activity {
                date: today,
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
        let results = query_activity(&db, 7).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].total_kcal, Some(2450.0));
        assert_eq!(results[0].active_kcal, Some(612.5));
        assert_eq!(results[0].bmr_kcal, Some(1838.0));
        assert_eq!(results[0].distance_meters, Some(8420.0));
    }

    #[test]
    fn stress_upsert_and_query_round_trips_durations() {
        let db = Database::open_memory().unwrap();
        let today = chrono::Local::now().date_naive().to_string();
        upsert_stress(
            &db,
            &Stress {
                date: today,
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
        let results = query_stress(&db, 7).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rest_seconds, Some(28800));
        assert_eq!(results[0].low_seconds, Some(18000));
        assert_eq!(results[0].medium_seconds, Some(4200));
        assert_eq!(results[0].high_seconds, None);
    }
}
