use anyhow::Result;
use rusqlite::params;
use rusqlite::OptionalExtension;

use super::Database;
use crate::models::{Activity, ExerciseSet, Heart, Recovery, Sleep, Stress, Workout};

// ---------------------------------------------------------------------------
// Sleep
// ---------------------------------------------------------------------------

pub fn upsert_sleep(db: &Database, s: &Sleep) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO sleep (date, total_seconds, deep_seconds, rem_seconds, light_seconds, awake_seconds, sleep_score, hrv_ms, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            s.date,
            s.total_seconds,
            s.deep_seconds,
            s.rem_seconds,
            s.light_seconds,
            s.awake_seconds,
            s.sleep_score,
            s.hrv_ms,
            s.source,
        ],
    )?;
    Ok(())
}

pub fn query_sleep(db: &Database, days: i32) -> Result<Vec<Sleep>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, total_seconds, deep_seconds, rem_seconds, light_seconds, awake_seconds, sleep_score, hrv_ms, source
         FROM sleep WHERE date >= date('now', ?1) ORDER BY date DESC",
    )?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map(params![modifier], |row| {
        Ok(Sleep {
            date: row.get(0)?,
            total_seconds: row.get(1)?,
            deep_seconds: row.get(2)?,
            rem_seconds: row.get(3)?,
            light_seconds: row.get(4)?,
            awake_seconds: row.get(5)?,
            sleep_score: row.get(6)?,
            hrv_ms: row.get(7)?,
            source: row.get(8)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_sleep(db: &Database, date: &str) -> Result<Option<Sleep>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, total_seconds, deep_seconds, rem_seconds, light_seconds, awake_seconds, sleep_score, hrv_ms, source
         FROM sleep WHERE date = ?1",
    )?;
    let result = stmt
        .query_row(params![date], |row| {
            Ok(Sleep {
                date: row.get(0)?,
                total_seconds: row.get(1)?,
                deep_seconds: row.get(2)?,
                rem_seconds: row.get(3)?,
                light_seconds: row.get(4)?,
                awake_seconds: row.get(5)?,
                sleep_score: row.get(6)?,
                hrv_ms: row.get(7)?,
                source: row.get(8)?,
            })
        })
        .optional()?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Heart
// ---------------------------------------------------------------------------

pub fn upsert_heart(db: &Database, h: &Heart) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO heart (date, resting_hr, max_hr, min_hr, hrv_avg, vo2_max, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![h.date, h.resting_hr, h.max_hr, h.min_hr, h.hrv_avg, h.vo2_max, h.source],
    )?;
    Ok(())
}

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

pub struct HeartBaselines {
    pub rhr_avg: Option<f64>,
    pub hrv_avg: Option<f64>,
}

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

pub fn upsert_activity(db: &Database, a: &Activity) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO activity (date, steps, active_minutes, floors, source)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![a.date, a.steps, a.active_minutes, a.floors, a.source],
    )?;
    Ok(())
}

pub fn query_activity(db: &Database, days: i32) -> Result<Vec<Activity>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, steps, active_minutes, floors, source
         FROM activity WHERE date >= date('now', ?1) ORDER BY date DESC",
    )?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map(params![modifier], |row| {
        Ok(Activity {
            date: row.get(0)?,
            steps: row.get(1)?,
            active_minutes: row.get(2)?,
            floors: row.get(3)?,
            source: row.get(4)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------------------
// Stress
// ---------------------------------------------------------------------------

pub fn upsert_stress(db: &Database, s: &Stress) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO stress (date, avg_stress, max_stress, source)
         VALUES (?1, ?2, ?3, ?4)",
        params![s.date, s.avg_stress, s.max_stress, s.source],
    )?;
    Ok(())
}

pub fn query_stress(db: &Database, days: i32) -> Result<Vec<Stress>> {
    let mut stmt = db.conn().prepare(
        "SELECT date, avg_stress, max_stress, source
         FROM stress WHERE date >= date('now', ?1) ORDER BY date DESC",
    )?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map(params![modifier], |row| {
        Ok(Stress {
            date: row.get(0)?,
            avg_stress: row.get(1)?,
            max_stress: row.get(2)?,
            source: row.get(3)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------------------
// Workouts
// ---------------------------------------------------------------------------

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

pub fn delete_exercise_sets_for_workout(db: &Database, workout_id: &str) -> Result<()> {
    db.conn().execute(
        "DELETE FROM exercise_sets WHERE workout_id = ?1",
        params![workout_id],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Sync State
// ---------------------------------------------------------------------------

pub fn get_sync_state(db: &Database, key: &str) -> Result<Option<String>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT value FROM sync_state WHERE key = ?1")?;
    let result = stmt
        .query_row(params![key], |row| row.get::<_, Option<String>>(0))
        .optional()?;
    Ok(result.flatten())
}

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
            source: "garmin".into(),
        };

        upsert_sleep(&db, &sleep).unwrap();
        let fetched = get_sleep(&db, "2026-03-01").unwrap().expect("should exist");
        assert_eq!(fetched.date, "2026-03-01");
        assert_eq!(fetched.total_seconds, 28800);
        assert_eq!(fetched.sleep_score, Some(82));
        assert!((fetched.hrv_ms.unwrap() - 45.5).abs() < f64::EPSILON);
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
            exercise_name: "bench press".into(),
            repetitions: Some(10),
            weight_kg: Some(80.0),
        };
        let set2 = ExerciseSet {
            id: None,
            workout_id: "w-001".into(),
            set_order: 2,
            exercise_category: Some("chest".into()),
            exercise_name: "bench press".into(),
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

        // Delete and verify
        delete_exercise_sets_for_workout(&db, "w-001").unwrap();
        let sets = query_exercise_sets(&db, "w-001").unwrap();
        assert!(sets.is_empty());
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
        upsert_heart(&db, &Heart {
            date: today.clone(),
            resting_hr: None,
            max_hr: None,
            min_hr: None,
            hrv_avg: None,
            vo2_max: Some(50.0),
            source: "garmin".into(),
        }).unwrap();
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
        upsert_heart(&db, &Heart {
            date: today.clone(),
            resting_hr: Some(58),
            max_hr: None,
            min_hr: None,
            hrv_avg: Some(50.0),
            vo2_max: Some(48.5),
            source: "garmin".into(),
        }).unwrap();
        let results = query_heart(&db, 7).unwrap();
        assert_eq!(results.len(), 1);
        assert!((results[0].vo2_max.unwrap() - 48.5).abs() < 0.01);
    }
}
