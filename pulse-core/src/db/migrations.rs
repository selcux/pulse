//! Schema migrations for the local SQLite database.
//!
//! Migrations are numbered and applied in order. The current schema version
//! is tracked in the `sync_state` table under the key `"schema_version"`.
//! [`run`] is called automatically by [`Database::open`](crate::db::Database::open).

use anyhow::{Context, Result};

use super::Database;

/// The schema version this build expects. Migrations up to this version are
/// applied automatically when the database is opened.
const CURRENT_VERSION: i32 = 7;

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

    if version < 6 {
        migrate_v6(db)?;
    }

    if version < 7 {
        migrate_v7(db)?;
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

/// Columns added by v7, as `(table, column, SQL type)`.
const V7_COLUMNS: [(&str, &str, &str); 12] = [
    ("sleep", "start_time", "TEXT"),
    ("sleep", "end_time", "TEXT"),
    ("sleep", "spo2_avg", "REAL"),
    ("sleep", "respiration_avg", "REAL"),
    ("stress", "rest_seconds", "INTEGER"),
    ("stress", "low_seconds", "INTEGER"),
    ("stress", "medium_seconds", "INTEGER"),
    ("stress", "high_seconds", "INTEGER"),
    ("activity", "total_kcal", "REAL"),
    ("activity", "active_kcal", "REAL"),
    ("activity", "bmr_kcal", "REAL"),
    ("activity", "distance_meters", "REAL"),
];

/// Fills the v7 columns from stored Garmin bodies. Must produce exactly what the Garmin mapping
/// in `providers::garmin` produces from the same body;
/// `migration_v7_backfill_matches_the_rust_mapping` pins that. Only rows with a stored body are
/// written; no row is inserted.
const V7_BACKFILL: &str = "
    UPDATE sleep SET
        start_time      = datetime(json_extract(r.body, '$.dailySleepDTO.sleepStartTimestampLocal') / 1000, 'unixepoch'),
        end_time        = datetime(json_extract(r.body, '$.dailySleepDTO.sleepEndTimestampLocal') / 1000, 'unixepoch'),
        spo2_avg        = json_extract(r.body, '$.dailySleepDTO.averageSpO2Value'),
        respiration_avg = json_extract(r.body, '$.dailySleepDTO.averageRespirationValue')
    FROM raw_responses AS r
    WHERE r.provider = 'garmin' AND r.endpoint = 'sleep' AND r.key = sleep.date;

    UPDATE stress SET
        rest_seconds   = CAST(json_extract(r.body, '$.restStressDuration') AS INTEGER),
        low_seconds    = CAST(json_extract(r.body, '$.lowStressDuration') AS INTEGER),
        medium_seconds = CAST(json_extract(r.body, '$.mediumStressDuration') AS INTEGER),
        high_seconds   = CAST(json_extract(r.body, '$.highStressDuration') AS INTEGER)
    FROM raw_responses AS r
    WHERE r.provider = 'garmin' AND r.endpoint = 'daily_summary' AND r.key = stress.date;

    UPDATE activity SET
        total_kcal      = json_extract(r.body, '$.totalKilocalories'),
        active_kcal     = json_extract(r.body, '$.activeKilocalories'),
        bmr_kcal        = json_extract(r.body, '$.bmrKilocalories'),
        distance_meters = json_extract(r.body, '$.totalDistanceMeters')
    FROM raw_responses AS r
    WHERE r.provider = 'garmin' AND r.endpoint = 'daily_summary' AND r.key = activity.date;
";

/// Adds the sleep, stress and activity columns of issues #2, #8, #9 and #11, and fills them for
/// every row whose Garmin body is stored in `raw_responses`. Rows without one (days synced before
/// 0.5.0, or with raw storage off) stay NULL until synced again.
///
/// One transaction, safe to run again: an existing column is not added twice and the backfill
/// writes the same values. That is needed beyond tests, because a 0.5 binary opening a v7
/// database stamps it back to 6, so the next 0.6 open runs this again.
pub(crate) fn migrate_v7(db: &Database) -> Result<()> {
    let tx = db.conn().unchecked_transaction()?;
    for (table, column, decl) in V7_COLUMNS {
        add_column_if_missing(&tx, table, column, decl)?;
    }
    tx.execute_batch(V7_BACKFILL)
        .context("Failed to run v7 migration")?;
    tx.commit().context("Failed to commit v7 migration")?;
    Ok(())
}

/// Adds `column` of type `decl` to `table` unless it is there already: SQLite has no
/// `ADD COLUMN IF NOT EXISTS`. The arguments are spliced into the SQL; pass constants only.
fn add_column_if_missing(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    decl: &str,
) -> Result<()> {
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info(?1) WHERE name = ?2",
            rusqlite::params![table, column],
            |row| row.get(0),
        )
        .with_context(|| format!("Failed to look up {table}.{column}"))?;
    if exists {
        return Ok(());
    }
    conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};"))
        .with_context(|| format!("Failed to add {table}.{column}"))
}

/// Creates `raw_responses`, which keeps each provider response body as received so the mapped
/// tables can be refilled without a re-sync. `CREATE TABLE IF NOT EXISTS`: safe to run again.
///
/// The `json_valid` check keeps a malformed body out of the table, because a later migration
/// that runs `json_extract` over every body would fail on it and stop the database from opening.
fn migrate_v6(db: &Database) -> Result<()> {
    db.conn()
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS raw_responses (
                provider   TEXT NOT NULL,
                endpoint   TEXT NOT NULL,
                key        TEXT NOT NULL,
                body       TEXT NOT NULL CHECK (json_valid(body)),
                fetched_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (provider, endpoint, key)
            );",
        )
        .context("Failed to run v6 migration")?;
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
        assert_eq!(CURRENT_VERSION, 7);
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
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        assert_eq!(set_rows(&db), empty_labels_converted());
    }

    #[test]
    fn v5_can_run_twice() {
        let db = v4_db_with_empty_labels();
        migrate_v5(&db).unwrap();
        migrate_v5(&db).unwrap();
        assert_eq!(set_rows(&db), empty_labels_converted());
    }

    /// A database as pulse 0.4.0 left it: schema version 5.
    fn v5_db() -> Database {
        let db = v4_db_with_empty_labels();
        migrate_v5(&db).unwrap();
        set_version(&db, 5).unwrap();
        db
    }

    #[test]
    fn v6_creates_raw_responses() {
        let db = v5_db();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        assert_eq!(set_rows(&db), empty_labels_converted());

        let mut stmt = db
            .conn()
            .prepare("PRAGMA table_info(raw_responses)")
            .unwrap();
        let columns: Vec<(String, String, bool, i32)> = stmt
            .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let expected: Vec<(String, String, bool, i32)> = [
            ("provider", "TEXT", true, 1),
            ("endpoint", "TEXT", true, 2),
            ("key", "TEXT", true, 3),
            ("body", "TEXT", true, 0),
            ("fetched_at", "TEXT", true, 0),
        ]
        .into_iter()
        .map(|(n, t, nn, pk)| (n.to_string(), t.to_string(), nn, pk))
        .collect();
        assert_eq!(columns, expected);
    }

    #[test]
    fn v6_can_run_twice() {
        let db = v5_db();
        migrate_v6(&db).unwrap();
        migrate_v6(&db).unwrap();
    }

    #[test]
    fn raw_responses_rejects_a_body_that_is_not_json() {
        let db = Database::open_memory().unwrap();
        let insert = |body: &str| {
            db.conn().execute(
                "INSERT INTO raw_responses (provider, endpoint, key, body) VALUES ('p', 'e', ?1, ?1)",
                [body],
            )
        };
        assert!(insert("<html>").is_err());
        assert!(insert("").is_err());
        assert!(insert("{}").is_ok());
    }

    /// A database as pulse 0.5.x left it: schema version 6, two days of Garmin rows without the
    /// v7 columns, and stored bodies for one of those days plus one day that has no rows.
    fn v6_db_with_garmin_rows() -> Database {
        let db = v5_db();
        migrate_v6(&db).unwrap();
        set_version(&db, 6).unwrap();
        db.conn()
            .execute_batch(
                r#"INSERT INTO sleep (date, total_seconds, deep_seconds, rem_seconds, light_seconds, awake_seconds)
                 VALUES ('2026-03-01', 28800, 7200, 5400, 14400, 1800),
                        ('2026-03-02', 27000, 6300, 5400, 14400, 900);
                 INSERT INTO stress (date, avg_stress, max_stress)
                 VALUES ('2026-03-01', 35, 78), ('2026-03-02', 30, 70);
                 INSERT INTO activity (date, steps)
                 VALUES ('2026-03-01', 12345), ('2026-03-02', 8000);
                 INSERT INTO raw_responses (provider, endpoint, key, body) VALUES
                 ('garmin', 'sleep', '2026-03-01', '{"dailySleepDTO":{"sleepTimeInSeconds":28800,"sleepStartTimestampLocal":1772320200999,"sleepEndTimestampLocal":1772348100000,"averageSpO2Value":95.0,"averageRespirationValue":14.5}}'),
                 ('garmin', 'daily_summary', '2026-03-01', '{"totalSteps":12345,"restStressDuration":28800,"lowStressDuration":18000,"mediumStressDuration":4200,"highStressDuration":null,"totalKilocalories":2450.0,"activeKilocalories":612.0,"bmrKilocalories":1838.0,"totalDistanceMeters":8420}'),
                 ('garmin', 'sleep', '2026-03-03', '{"dailySleepDTO":{"sleepTimeInSeconds":28800,"sleepStartTimestampLocal":1772320200999,"sleepEndTimestampLocal":1772348100000,"averageSpO2Value":95.0,"averageRespirationValue":14.5}}'),
                 ('garmin', 'daily_summary', '2026-03-03', '{"totalSteps":12345,"restStressDuration":28800,"lowStressDuration":18000,"mediumStressDuration":4200,"highStressDuration":null,"totalKilocalories":2450.0,"activeKilocalories":612.0,"bmrKilocalories":1838.0,"totalDistanceMeters":8420}'),
                 ('garmin', 'hrv', '2026-03-02', '{"restStressDuration":1,"totalKilocalories":1.0,"dailySleepDTO":{"averageSpO2Value":1.0}}');"#,
            )
            .unwrap();
        db
    }

    /// `PRAGMA table_info(table)` as `(name, declared type)` pairs.
    fn columns_of(db: &Database, table: &str) -> Vec<(String, String)> {
        let mut stmt = db
            .conn()
            .prepare("SELECT name, type FROM pragma_table_info(?1)")
            .unwrap();
        stmt.query_map([table], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    fn pairs(columns: &[(&str, &str)]) -> Vec<(String, String)> {
        columns
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect()
    }

    #[test]
    fn v7_adds_the_new_columns() {
        let db = v6_db_with_garmin_rows();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);

        assert_eq!(
            columns_of(&db, "sleep"),
            pairs(&[
                ("date", "TEXT"),
                ("total_seconds", "INTEGER"),
                ("deep_seconds", "INTEGER"),
                ("rem_seconds", "INTEGER"),
                ("light_seconds", "INTEGER"),
                ("awake_seconds", "INTEGER"),
                ("sleep_score", "INTEGER"),
                ("hrv_ms", "REAL"),
                ("source", "TEXT"),
                ("start_time", "TEXT"),
                ("end_time", "TEXT"),
                ("spo2_avg", "REAL"),
                ("respiration_avg", "REAL"),
            ])
        );
        assert_eq!(
            columns_of(&db, "stress"),
            pairs(&[
                ("date", "TEXT"),
                ("avg_stress", "INTEGER"),
                ("max_stress", "INTEGER"),
                ("source", "TEXT"),
                ("rest_seconds", "INTEGER"),
                ("low_seconds", "INTEGER"),
                ("medium_seconds", "INTEGER"),
                ("high_seconds", "INTEGER"),
            ])
        );
        assert_eq!(
            columns_of(&db, "activity"),
            pairs(&[
                ("date", "TEXT"),
                ("steps", "INTEGER"),
                ("active_minutes", "INTEGER"),
                ("floors", "INTEGER"),
                ("source", "TEXT"),
                ("total_kcal", "REAL"),
                ("active_kcal", "REAL"),
                ("bmr_kcal", "REAL"),
                ("distance_meters", "REAL"),
            ])
        );
    }

    type SleepExtras = (Option<String>, Option<String>, Option<f64>, Option<f64>);
    type StressExtras = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);
    type ActivityExtras = (Option<f64>, Option<f64>, Option<f64>, Option<f64>);

    fn sleep_extras(db: &Database, date: &str) -> SleepExtras {
        db.conn()
            .query_row(
                "SELECT start_time, end_time, spo2_avg, respiration_avg FROM sleep WHERE date = ?1",
                [date],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
    }

    fn stress_extras(db: &Database, date: &str) -> StressExtras {
        db.conn()
            .query_row(
                "SELECT rest_seconds, low_seconds, medium_seconds, high_seconds FROM stress WHERE date = ?1",
                [date],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
    }

    fn activity_extras(db: &Database, date: &str) -> ActivityExtras {
        db.conn()
            .query_row(
                "SELECT total_kcal, active_kcal, bmr_kcal, distance_meters FROM activity WHERE date = ?1",
                [date],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
    }

    fn count(db: &Database, table: &str) -> i64 {
        db.conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// Asserts the values the fixture's bodies produce, and that the other rows stayed empty.
    fn assert_v7_backfilled(db: &Database) {
        assert_eq!(
            sleep_extras(db, "2026-03-01"),
            (
                text("2026-02-28 23:10:00"),
                text("2026-03-01 06:55:00"),
                Some(95.0),
                Some(14.5)
            )
        );
        assert_eq!(
            stress_extras(db, "2026-03-01"),
            (Some(28800), Some(18000), Some(4200), None)
        );
        assert_eq!(
            activity_extras(db, "2026-03-01"),
            (Some(2450.0), Some(612.0), Some(1838.0), Some(8420.0))
        );

        // The decoy under the `hrv` endpoint was not used.
        assert_eq!(sleep_extras(db, "2026-03-02"), (None, None, None, None));
        assert_eq!(stress_extras(db, "2026-03-02"), (None, None, None, None));
        assert_eq!(activity_extras(db, "2026-03-02"), (None, None, None, None));

        // No row was inserted for the day that only has bodies.
        assert_eq!(count(db, "sleep"), 2);
        assert_eq!(count(db, "stress"), 2);
        assert_eq!(count(db, "activity"), 2);

        // The old columns are untouched.
        let avg_stress: Option<i64> = db
            .conn()
            .query_row(
                "SELECT avg_stress FROM stress WHERE date = '2026-03-01'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(avg_stress, Some(35));
        assert_eq!(set_rows(db), empty_labels_converted());
    }

    #[test]
    fn v7_fills_rows_that_have_a_stored_body() {
        let db = v6_db_with_garmin_rows();
        run(&db).unwrap();
        assert_v7_backfilled(&db);
    }

    #[test]
    fn v7_runs_with_no_stored_bodies() {
        let db = v6_db_with_garmin_rows();
        db.conn().execute("DELETE FROM raw_responses", []).unwrap();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        for date in ["2026-03-01", "2026-03-02"] {
            assert_eq!(sleep_extras(&db, date), (None, None, None, None));
            assert_eq!(stress_extras(&db, date), (None, None, None, None));
            assert_eq!(activity_extras(&db, date), (None, None, None, None));
        }
    }

    #[test]
    fn v7_can_run_twice() {
        let db = v6_db_with_garmin_rows();
        migrate_v7(&db).unwrap();
        migrate_v7(&db).unwrap();
        assert_v7_backfilled(&db);
    }
}
