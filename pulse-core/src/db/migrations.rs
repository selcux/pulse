//! Schema migrations for the local SQLite database.
//!
//! Migrations are numbered and applied in order. The current schema version
//! is tracked in the `sync_state` table under the key `"schema_version"`.
//! [`run`] is called automatically by [`Database::open`](crate::db::Database::open).

use anyhow::{Context, Result};

use super::Database;

/// The schema version this build expects. Migrations up to this version are
/// applied automatically when the database is opened.
const CURRENT_VERSION: i32 = 5;

/// Apply all pending schema migrations up to `CURRENT_VERSION`.
///
/// Safe to call on an already-migrated database — already-applied migrations
/// are skipped. Called automatically by
/// [`Database::open`](crate::db::Database::open).
pub fn run(db: &Database) -> Result<()> {
    let conn = db.conn();

    // Bootstrap: ensure sync_state table exists so we can track schema version.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_state (
            key TEXT PRIMARY KEY,
            value TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .context("Failed to create sync_state table")?;

    let version = current_version(db)?;

    if version < 1 {
        migrate_v1(db)?;
    }

    if version < 2 {
        migrate_v2(db)?;
    }

    if version < 3 {
        migrate_v3(db)?;
    }

    if version < 4 {
        migrate_v4(db)?;
    }

    if version < 5 {
        migrate_v5(db)?;
    }

    // Stamp the version after all migrations succeed.
    set_version(db, CURRENT_VERSION)?;

    Ok(())
}

fn current_version(db: &Database) -> Result<i32> {
    let conn = db.conn();
    let mut stmt = conn.prepare("SELECT value FROM sync_state WHERE key = 'schema_version'")?;
    let version = stmt
        .query_row([], |row| {
            let v: Option<String> = row.get(0)?;
            Ok(v.and_then(|s| s.parse::<i32>().ok()).unwrap_or(0))
        })
        .unwrap_or(0);
    Ok(version)
}

fn set_version(db: &Database, version: i32) -> Result<()> {
    db.conn().execute(
        "INSERT OR REPLACE INTO sync_state (key, value, updated_at) VALUES ('schema_version', ?1, datetime('now'))",
        rusqlite::params![version.to_string()],
    )?;
    Ok(())
}

/// Turns empty-string `exercise_name` and `exercise_category` values into NULL.
///
/// Sets written before `garmin_label` filtered empty labels may hold `''`, which displays as
/// a blank name instead of falling back to the category. After v4 both columns are nullable,
/// so no rebuild is needed. Only `''` is converted, as `garmin_label` does not trim either.
/// The update matches nothing on a database that already has it, so it is safe to run again.
fn migrate_v5(db: &Database) -> Result<()> {
    db.conn()
        .execute_batch(
            "UPDATE exercise_sets
             SET exercise_name = NULLIF(exercise_name, ''),
                 exercise_category = NULLIF(exercise_category, '')
             WHERE exercise_name = '' OR exercise_category = '';",
        )
        .context("Failed to run v5 migration")?;
    Ok(())
}

/// Makes `exercise_sets.exercise_name` nullable and turns the stored `Unknown` placeholders
/// in `exercise_name` and `exercise_category` into NULL.
///
/// SQLite cannot drop a NOT NULL constraint in place, so the table is rebuilt. The rebuild
/// runs in one transaction and is safe to run again on a database that already has it.
fn migrate_v4(db: &Database) -> Result<()> {
    let tx = db.conn().unchecked_transaction()?;
    // `defer_foreign_keys` moves the foreign-key check to COMMIT, so a set whose workout row
    // is missing does not fail the copy: the copy and its dropped original cancel out.
    tx.execute_batch(
        "
        PRAGMA defer_foreign_keys=ON;

        DROP TABLE IF EXISTS exercise_sets_new;

        CREATE TABLE exercise_sets_new (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            workout_id TEXT NOT NULL REFERENCES workouts(id),
            set_order INTEGER NOT NULL,
            exercise_category TEXT,
            exercise_name TEXT,
            repetitions INTEGER,
            weight_kg REAL
        );

        INSERT INTO exercise_sets_new
            (id, workout_id, set_order, exercise_category, exercise_name, repetitions, weight_kg)
        SELECT id, workout_id, set_order,
               NULLIF(exercise_category, 'Unknown'), NULLIF(exercise_name, 'Unknown'),
               repetitions, weight_kg
        FROM exercise_sets;

        DROP TABLE exercise_sets;

        ALTER TABLE exercise_sets_new RENAME TO exercise_sets;
        ",
    )
    .context("Failed to run v4 migration")?;
    tx.commit().context("Failed to commit v4 migration")?;
    Ok(())
}

fn migrate_v3(db: &Database) -> Result<()> {
    db.conn()
        .execute_batch("ALTER TABLE heart ADD COLUMN vo2_max REAL;")
        .context("Failed to run v3 migration")?;
    Ok(())
}

fn migrate_v2(db: &Database) -> Result<()> {
    db.conn()
        .execute_batch("ALTER TABLE workouts ADD COLUMN name TEXT;")
        .context("Failed to run v2 migration")?;
    Ok(())
}

fn migrate_v1(db: &Database) -> Result<()> {
    db.conn()
        .execute_batch(
            "
        CREATE TABLE IF NOT EXISTS sleep (
            date TEXT PRIMARY KEY,
            total_seconds INTEGER NOT NULL,
            deep_seconds INTEGER NOT NULL,
            rem_seconds INTEGER NOT NULL,
            light_seconds INTEGER NOT NULL,
            awake_seconds INTEGER NOT NULL,
            sleep_score INTEGER,
            hrv_ms REAL,
            source TEXT NOT NULL DEFAULT 'garmin'
        );

        CREATE TABLE IF NOT EXISTS heart (
            date TEXT PRIMARY KEY,
            resting_hr INTEGER,
            max_hr INTEGER,
            min_hr INTEGER,
            hrv_avg REAL,
            source TEXT NOT NULL DEFAULT 'garmin'
        );

        CREATE TABLE IF NOT EXISTS recovery (
            date TEXT PRIMARY KEY,
            body_battery_charged INTEGER,
            body_battery_drained INTEGER,
            body_battery_peak INTEGER,
            body_battery_low INTEGER,
            source TEXT NOT NULL DEFAULT 'garmin'
        );

        CREATE TABLE IF NOT EXISTS activity (
            date TEXT PRIMARY KEY,
            steps INTEGER,
            active_minutes INTEGER,
            floors INTEGER,
            source TEXT NOT NULL DEFAULT 'garmin'
        );

        CREATE TABLE IF NOT EXISTS stress (
            date TEXT PRIMARY KEY,
            avg_stress INTEGER,
            max_stress INTEGER,
            source TEXT NOT NULL DEFAULT 'garmin'
        );

        CREATE TABLE IF NOT EXISTS workouts (
            id TEXT PRIMARY KEY,
            start_time TEXT NOT NULL,
            duration_seconds INTEGER NOT NULL,
            activity_type TEXT NOT NULL,
            calories INTEGER,
            avg_hr INTEGER,
            max_hr INTEGER,
            source TEXT NOT NULL DEFAULT 'intervals'
        );

        CREATE TABLE IF NOT EXISTS exercise_sets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            workout_id TEXT NOT NULL REFERENCES workouts(id),
            set_order INTEGER NOT NULL,
            exercise_category TEXT,
            exercise_name TEXT NOT NULL,
            repetitions INTEGER,
            weight_kg REAL
        );
        ",
        )
        .context("Failed to run v1 migration")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// A database as pulse 0.2.0 left it: schema version 3, one workout, six sets.
    fn v3_db() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        let db = Database { conn };
        db.conn()
            .execute_batch(
                "CREATE TABLE sync_state (
                    key TEXT PRIMARY KEY,
                    value TEXT,
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );",
            )
            .unwrap();
        migrate_v1(&db).unwrap();
        migrate_v2(&db).unwrap();
        migrate_v3(&db).unwrap();
        set_version(&db, 3).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO workouts (id, start_time, duration_seconds, activity_type)
                 VALUES ('w1', '2026-01-02T07:30:00', 3600, 'WeightTraining');
                 INSERT INTO exercise_sets
                     (workout_id, set_order, exercise_category, exercise_name, repetitions, weight_kg)
                 VALUES
                     ('w1', 1, 'Chest', 'Bench Press', 10, 60.0),
                     ('w1', 2, 'Unknown', 'Unknown', 8, NULL),
                     ('w1', 3, NULL, 'Unknown', 5, 20.0),
                     ('w1', 4, 'Unknown', 'Squat', 5, 100.0),
                     ('w1', 5, 'Legs', 'Unknown Exercise', 12, NULL),
                     ('w1', 6, 'unknown', 'UNKNOWN', 3, NULL);",
            )
            .unwrap();
        db
    }

    /// `(id, set_order, exercise_category, exercise_name, repetitions, weight_kg)` of one set.
    type SetRow = (
        i64,
        i32,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<f64>,
    );

    fn set_rows(db: &Database) -> Vec<SetRow> {
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id, set_order, exercise_category, exercise_name, repetitions, weight_kg
                 FROM exercise_sets ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    }

    fn text(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    /// What `v3_db` holds after the v4 migration.
    fn converted() -> Vec<SetRow> {
        vec![
            (
                1,
                1,
                text("Chest"),
                text("Bench Press"),
                Some(10),
                Some(60.0),
            ),
            (2, 2, None, None, Some(8), None),
            (3, 3, None, None, Some(5), Some(20.0)),
            (4, 4, None, text("Squat"), Some(5), Some(100.0)),
            (5, 5, text("Legs"), text("Unknown Exercise"), Some(12), None),
            (6, 6, text("unknown"), text("UNKNOWN"), Some(3), None),
        ]
    }

    #[test]
    fn fresh_database_is_at_current_version() {
        let db = Database::open_memory().unwrap();
        assert_eq!(CURRENT_VERSION, 5);
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
    }

    #[test]
    fn v4_converts_unknown_placeholders_to_null() {
        let db = v3_db();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        assert_eq!(set_rows(&db), converted());
    }

    #[test]
    fn v4_makes_exercise_name_nullable_and_keeps_the_other_constraints() {
        let db = v3_db();
        run(&db).unwrap();

        let mut stmt = db
            .conn()
            .prepare("PRAGMA table_info(exercise_sets)")
            .unwrap();
        let columns: Vec<(String, String, bool, bool)> = stmt
            .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let expected: Vec<(String, String, bool, bool)> = [
            ("id", "INTEGER", false, true),
            ("workout_id", "TEXT", true, false),
            ("set_order", "INTEGER", true, false),
            ("exercise_category", "TEXT", false, false),
            ("exercise_name", "TEXT", false, false),
            ("repetitions", "INTEGER", false, false),
            ("weight_kg", "REAL", false, false),
        ]
        .into_iter()
        .map(|(n, t, nn, pk)| (n.to_string(), t.to_string(), nn, pk))
        .collect();
        assert_eq!(columns, expected);

        // The foreign key to workouts is still enforced.
        let orphan = db.conn().execute(
            "INSERT INTO exercise_sets (workout_id, set_order) VALUES ('missing', 1)",
            [],
        );
        assert!(orphan.is_err());

        // Only an AUTOINCREMENT table has a row in sqlite_sequence.
        let seq: i64 = db
            .conn()
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = 'exercise_sets'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seq, 6);

        let leftover: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'exercise_sets_new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0);
    }

    #[test]
    fn v4_can_run_twice() {
        let db = v3_db();
        migrate_v4(&db).unwrap();
        migrate_v4(&db).unwrap();
        assert_eq!(set_rows(&db), converted());
    }

    #[test]
    fn v4_keeps_a_set_whose_workout_is_missing() {
        let db = v3_db();
        db.conn()
            .execute_batch(
                "PRAGMA foreign_keys=OFF;
                 INSERT INTO exercise_sets (workout_id, set_order, exercise_name) VALUES ('gone', 1, 'Unknown');
                 PRAGMA foreign_keys=ON;",
            )
            .unwrap();
        run(&db).unwrap();
        let rows = set_rows(&db);
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[6], (7, 1, None, None, None, None));
    }

    #[test]
    fn v4_rolls_back_when_a_step_fails() {
        let db = v3_db();
        // A view on the table makes the final rename fail, after the old table was dropped.
        db.conn()
            .execute_batch("CREATE VIEW set_names AS SELECT exercise_name FROM exercise_sets;")
            .unwrap();

        assert!(run(&db).is_err());

        assert_eq!(current_version(&db).unwrap(), 3);
        assert!(db.conn().is_autocommit());
        assert_eq!(set_rows(&db).len(), 6);
        let not_null: bool = db
            .conn()
            .query_row(
                r#"SELECT "notnull" FROM pragma_table_info('exercise_sets') WHERE name = 'exercise_name'"#,
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(not_null);
    }

    /// A database as pulse 0.3.1 left it: schema version 4, with empty labels from before
    /// `garmin_label` dropped them.
    fn v4_db_with_empty_labels() -> Database {
        let db = v3_db();
        migrate_v4(&db).unwrap();
        set_version(&db, 4).unwrap();
        db.conn()
            .execute_batch(
                "DELETE FROM exercise_sets;
                 INSERT INTO exercise_sets
                     (id, workout_id, set_order, exercise_category, exercise_name, repetitions, weight_kg)
                 VALUES
                     (1, 'w1', 1, 'Chest', 'Bench Press', 10, 60.0),
                     (2, 'w1', 2, 'Chest', '', 8, NULL),
                     (3, 'w1', 3, '', 'Squat', 5, 100.0),
                     (4, 'w1', 4, '', '', 5, 20.0),
                     (5, 'w1', 5, NULL, NULL, 12, NULL),
                     (6, 'w1', 6, ' ', ' ', 3, NULL);",
            )
            .unwrap();
        db
    }

    /// What `v4_db_with_empty_labels` holds after the v5 migration. A label of only
    /// whitespace is kept: `garmin_label` does not trim either.
    fn empty_labels_converted() -> Vec<SetRow> {
        vec![
            (
                1,
                1,
                text("Chest"),
                text("Bench Press"),
                Some(10),
                Some(60.0),
            ),
            (2, 2, text("Chest"), None, Some(8), None),
            (3, 3, None, text("Squat"), Some(5), Some(100.0)),
            (4, 4, None, None, Some(5), Some(20.0)),
            (5, 5, None, None, Some(12), None),
            (6, 6, text(" "), text(" "), Some(3), None),
        ]
    }

    #[test]
    fn v5_converts_empty_labels_to_null() {
        let db = v4_db_with_empty_labels();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), 5);
        assert_eq!(set_rows(&db), empty_labels_converted());
    }

    #[test]
    fn v5_can_run_twice() {
        let db = v4_db_with_empty_labels();
        migrate_v5(&db).unwrap();
        migrate_v5(&db).unwrap();
        assert_eq!(set_rows(&db), empty_labels_converted());
    }
}
