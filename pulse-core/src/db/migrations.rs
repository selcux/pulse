//! Schema migrations for the local SQLite database.
//!
//! Migrations are numbered and applied in order. The current schema version
//! is tracked in the `sync_state` table under the key `"schema_version"`.
//! [`run`] is called automatically by [`Database::open`](crate::db::Database::open).

use anyhow::{Context, Result};

use super::Database;

/// The schema version this build expects. Migrations up to this version are
/// applied automatically when the database is opened.
const CURRENT_VERSION: i32 = 12;

/// Apply all pending schema migrations up to `CURRENT_VERSION`.
///
/// Safe to call on an already-migrated database — already-applied migrations
/// are skipped. Called automatically by
/// [`Database::open`](crate::db::Database::open).
///
/// # Errors
///
/// Fails without writing the version when the stored schema version is above
/// `CURRENT_VERSION`, i.e. a newer pulse has already migrated the database.
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

    // A newer pulse migrated this database. Going on would stamp it back down to
    // `CURRENT_VERSION` and let syncs blank the columns this build does not know (issue #21).
    if version > CURRENT_VERSION {
        anyhow::bail!(
            "the database is at schema version {version}, but this pulse only supports up to \
             version {CURRENT_VERSION}. Upgrade pulse (cargo install pulse-cli) to open it."
        );
    }

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

    if version < 8 {
        migrate_v8(db)?;
    }

    if version < 9 {
        migrate_v9(db)?;
    }

    if version < 10 {
        migrate_v10(db)?;
    }

    if version < 11 {
        migrate_v11(db)?;
    }

    if version < 12 {
        migrate_v12(db)?;
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

/// Columns added by v12, as `(table, column, SQL type)`.
const V12_COLUMNS: [(&str, &str, &str); 1] = [("workouts", "hr_zone_max_bpm", "TEXT")];

/// Fills `workouts.hr_zone_max_bpm` from stored Intervals.icu activity bodies. Must store exactly
/// what `IntervalsProvider::map_workout` stores for the same body: the `icu_hr_zones` array as
/// minified JSON text when it is not empty, every element is a JSON integer from 1 to i32::MAX,
/// and each element is above the one before (the self-join on `key + 1`); otherwise NULL. The
/// upper bound also rejects an integer beyond i64, which `json_each` returns as REAL.
/// `migration_v12_backfill_matches_the_rust_mapping` pins that. Only rows with a stored body
/// are written; no row is inserted.
const V12_BACKFILL: &str = "
    UPDATE workouts SET
        hr_zone_max_bpm = CASE
            WHEN json_type(r.body, '$.icu_hr_zones') = 'array'
             AND json_array_length(r.body, '$.icu_hr_zones') > 0
             AND NOT EXISTS (SELECT 1 FROM json_each(r.body, '$.icu_hr_zones') AS z
                             WHERE z.type <> 'integer' OR z.value NOT BETWEEN 1 AND 2147483647)
             AND NOT EXISTS (SELECT 1 FROM json_each(r.body, '$.icu_hr_zones') AS a
                             JOIN json_each(r.body, '$.icu_hr_zones') AS b ON b.key = a.key + 1
                             WHERE b.value <= a.value)
            THEN json_extract(r.body, '$.icu_hr_zones') END
    FROM raw_responses AS r
    WHERE r.provider = 'intervals' AND r.endpoint = 'activity' AND r.key = workouts.id;
";

/// Adds `workouts.hr_zone_max_bpm` (issue #23) and fills it for every workout whose
/// Intervals.icu body is stored in `raw_responses`. Workouts without one (synced before 0.5.0,
/// or with raw storage off) stay NULL until synced again.
///
/// One transaction, safe to run again for the same reason as `migrate_v7`.
pub(crate) fn migrate_v12(db: &Database) -> Result<()> {
    let tx = db.conn().unchecked_transaction()?;
    for (table, column, decl) in V12_COLUMNS {
        add_column_if_missing(&tx, table, column, decl)?;
    }
    tx.execute_batch(V12_BACKFILL)
        .context("Failed to run v12 migration")?;
    tx.commit().context("Failed to commit v12 migration")?;
    Ok(())
}

/// Creates `wellness` (issue #4): one row per day of Intervals.icu wellness data. There is no
/// backfill, because pulse stored no wellness bodies before this version.
/// `CREATE TABLE IF NOT EXISTS`: safe to run again.
fn migrate_v11(db: &Database) -> Result<()> {
    db.conn()
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS wellness (
                date TEXT NOT NULL PRIMARY KEY,
                weight_kg REAL,
                body_fat_percent REAL,
                ctl REAL,
                atl REAL,
                source TEXT NOT NULL DEFAULT 'intervals'
            );",
        )
        .context("Failed to run v11 migration")?;
    Ok(())
}

/// Columns added by v10, as `(table, column, SQL type)`.
const V10_COLUMNS: [(&str, &str, &str); 1] = [("workouts", "training_load", "INTEGER")];

/// Fills `workouts.training_load` from stored Intervals.icu activity bodies. Must store exactly
/// what `IntervalsProvider::map_workout` stores for the same body: a JSON number truncated toward
/// zero, kept from 1 to i32::MAX, otherwise NULL. The CAST matters: INTEGER affinity would keep a
/// fractional value as REAL, which `query_workouts` cannot read. Only rows with a stored body are
/// written; no row is inserted. `migration_v10_backfill_matches_the_rust_mapping` pins that.
const V10_BACKFILL: &str = "
    UPDATE workouts SET
        training_load = CASE WHEN json_type(r.body, '$.icu_training_load') IN ('integer', 'real')
                              AND CAST(json_extract(r.body, '$.icu_training_load') AS INTEGER)
                                  BETWEEN 1 AND 2147483647
                             THEN CAST(json_extract(r.body, '$.icu_training_load') AS INTEGER) END
    FROM raw_responses AS r
    WHERE r.provider = 'intervals' AND r.endpoint = 'activity' AND r.key = workouts.id;
";

/// Adds `workouts.training_load` (issue #7) and fills it for every workout whose Intervals.icu
/// body is stored in `raw_responses`. Workouts without one (synced before 0.5.0, or with raw
/// storage off) stay NULL until synced again.
///
/// One transaction, safe to run again for the same reason as `migrate_v7`.
pub(crate) fn migrate_v10(db: &Database) -> Result<()> {
    let tx = db.conn().unchecked_transaction()?;
    for (table, column, decl) in V10_COLUMNS {
        add_column_if_missing(&tx, table, column, decl)?;
    }
    tx.execute_batch(V10_BACKFILL)
        .context("Failed to run v10 migration")?;
    tx.commit().context("Failed to commit v10 migration")?;
    Ok(())
}

/// Columns added by v9, as `(table, column, SQL type)`.
const V9_COLUMNS: [(&str, &str, &str); 1] = [("workouts", "hr_zone_seconds", "TEXT")];

/// Fills `workouts.hr_zone_seconds` from stored Intervals.icu activity bodies. Must store exactly
/// what `IntervalsProvider::map_workout` stores for the same body: the `icu_hr_zone_times` array
/// as minified JSON text when every element is a JSON integer within i64 and not negative and at
/// least one is above zero, otherwise NULL. A JSON integer beyond i64 comes out of `json_each` as
/// REAL, hence the `typeof` test. `migration_v9_backfill_matches_the_rust_mapping` pins that.
/// Only rows with a stored body are written; no row is inserted.
const V9_BACKFILL: &str = "
    UPDATE workouts SET
        hr_zone_seconds = CASE
            WHEN json_type(r.body, '$.icu_hr_zone_times') = 'array'
             AND NOT EXISTS (SELECT 1 FROM json_each(r.body, '$.icu_hr_zone_times') AS z
                             WHERE z.type <> 'integer' OR typeof(z.value) <> 'integer' OR z.value < 0)
             AND EXISTS (SELECT 1 FROM json_each(r.body, '$.icu_hr_zone_times') AS z
                         WHERE z.value > 0)
            THEN json_extract(r.body, '$.icu_hr_zone_times') END
    FROM raw_responses AS r
    WHERE r.provider = 'intervals' AND r.endpoint = 'activity' AND r.key = workouts.id;
";

/// Adds `workouts.hr_zone_seconds` (issue #6) and fills it for every workout whose Intervals.icu
/// body is stored in `raw_responses`. Workouts without one (synced before 0.5.0, or with raw
/// storage off) stay NULL until synced again.
///
/// One transaction, safe to run again for the same reason as `migrate_v7`.
pub(crate) fn migrate_v9(db: &Database) -> Result<()> {
    let tx = db.conn().unchecked_transaction()?;
    for (table, column, decl) in V9_COLUMNS {
        add_column_if_missing(&tx, table, column, decl)?;
    }
    tx.execute_batch(V9_BACKFILL)
        .context("Failed to run v9 migration")?;
    tx.commit().context("Failed to commit v9 migration")?;
    Ok(())
}

/// Columns added by v8, as `(table, column, SQL type)`.
const V8_COLUMNS: [(&str, &str, &str); 2] = [
    ("workouts", "distance_meters", "REAL"),
    ("workouts", "avg_speed_mps", "REAL"),
];

/// Fills the v8 columns from stored Intervals.icu activity bodies. Must produce exactly what
/// `IntervalsProvider::map_workout` produces from the same body: a value that is not above zero
/// is NULL, and so is a JSON string, which serde rejects (SQLite would rank TEXT above any
/// number). `migration_v8_backfill_matches_the_rust_mapping` pins that. Only rows with a stored
/// body are written; no row is inserted.
const V8_BACKFILL: &str = "
    UPDATE workouts SET
        distance_meters = CASE WHEN json_type(r.body, '$.distance') IN ('real', 'integer')
                                AND json_extract(r.body, '$.distance') > 0
                               THEN json_extract(r.body, '$.distance') END,
        avg_speed_mps   = CASE WHEN json_type(r.body, '$.average_speed') IN ('real', 'integer')
                                AND json_extract(r.body, '$.average_speed') > 0
                               THEN json_extract(r.body, '$.average_speed') END
    FROM raw_responses AS r
    WHERE r.provider = 'intervals' AND r.endpoint = 'activity' AND r.key = workouts.id;
";

/// Adds `workouts.distance_meters` and `workouts.avg_speed_mps` (issue #5), and fills them for
/// every workout whose Intervals.icu body is stored in `raw_responses`. Workouts without one
/// (synced before 0.5.0, or with raw storage off) stay NULL until synced again.
///
/// One transaction, safe to run again for the same reason as `migrate_v7`.
pub(crate) fn migrate_v8(db: &Database) -> Result<()> {
    let tx = db.conn().unchecked_transaction()?;
    for (table, column, decl) in V8_COLUMNS {
        add_column_if_missing(&tx, table, column, decl)?;
    }
    tx.execute_batch(V8_BACKFILL)
        .context("Failed to run v8 migration")?;
    tx.commit().context("Failed to commit v8 migration")?;
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
        assert_eq!(CURRENT_VERSION, 12);
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
    }

    #[test]
    fn refuses_a_database_from_a_newer_pulse() {
        let db = Database::open_memory().unwrap();
        let newer = CURRENT_VERSION + 1;
        set_version(&db, newer).unwrap();

        let message = run(&db).unwrap_err().to_string();

        assert!(message.contains(&format!("version {newer}")), "{message}");
        assert!(
            message.contains(&format!("version {CURRENT_VERSION}")),
            "{message}"
        );
        assert!(message.contains("Upgrade pulse"), "{message}");
        assert_eq!(
            current_version(&db).unwrap(),
            newer,
            "the stored version must not be lowered"
        );
    }

    #[test]
    fn opens_a_database_at_the_current_version_again() {
        let db = Database::open_memory().unwrap();
        run(&db).unwrap();
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

    /// A database as pulse 0.6 to 0.8 left it: schema version 7, workouts without the v8 columns,
    /// and stored Intervals.icu activity bodies for some of them.
    fn v7_db_with_workouts() -> Database {
        let db = v6_db_with_garmin_rows();
        migrate_v7(&db).unwrap();
        set_version(&db, 7).unwrap();
        db.conn()
            .execute_batch(
                r#"INSERT INTO workouts (id, start_time, duration_seconds, activity_type) VALUES
                 ('i201', '2026-10-13T07:00:00', 3540, 'Walk'),
                 ('i202', '2026-10-14T18:00:00', 3600, 'WeightTraining'),
                 ('i203', '2026-10-15T07:00:00', 1800, 'Run'),
                 ('i205', '2026-10-15T09:00:00', 1200, 'Walk'),
                 ('i206', '2026-10-15T10:00:00', 600, 'Walk');
                 INSERT INTO raw_responses (provider, endpoint, key, body) VALUES
                 ('intervals', 'activity', 'i201', '{"id":"i201","type":"Walk","distance":5043.21,"average_speed":1.417,"pace":1.4375}'),
                 ('intervals', 'activity', 'i202', '{"id":"i202","type":"WeightTraining","distance":0,"average_speed":0.0}'),
                 ('intervals', 'activity', 'i205', '{"id":"i205","type":"Walk","distance":1670,"average_speed":null}'),
                 ('intervals', 'activity', 'i206', '{"id":"i206","type":"Walk","distance":"5000","average_speed":"1.4"}'),
                 ('intervals', 'activity', 'i204','{"id":"i204","type":"Walk","distance":1702.5,"average_speed":1.3}'),
                 ('garmin', 'activity', 'i203', '{"distance":1.0,"average_speed":1.0}');"#,
            )
            .unwrap();
        db
    }

    /// `(distance_meters, avg_speed_mps)` of one workout.
    fn workout_extras(db: &Database, id: &str) -> (Option<f64>, Option<f64>) {
        db.conn()
            .query_row(
                "SELECT distance_meters, avg_speed_mps FROM workouts WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn v8_adds_the_workout_columns() {
        let db = v7_db_with_workouts();
        migrate_v8(&db).unwrap();

        assert_eq!(
            columns_of(&db, "workouts"),
            pairs(&[
                ("id", "TEXT"),
                ("start_time", "TEXT"),
                ("duration_seconds", "INTEGER"),
                ("activity_type", "TEXT"),
                ("calories", "INTEGER"),
                ("avg_hr", "INTEGER"),
                ("max_hr", "INTEGER"),
                ("source", "TEXT"),
                ("name", "TEXT"),
                ("distance_meters", "REAL"),
                ("avg_speed_mps", "REAL"),
            ])
        );
    }

    /// Asserts the values the fixture's bodies produce, and that the other rows stayed empty.
    fn assert_v8_backfilled(db: &Database) {
        assert_eq!(
            workout_extras(db, "i201"),
            (Some(5043.21), Some(1.417)),
            "a stored body fills both columns"
        );
        assert_eq!(
            workout_extras(db, "i202"),
            (None, None),
            "zeros become NULL"
        );
        assert_eq!(
            workout_extras(db, "i203"),
            (None, None),
            "the decoy under another provider was not used"
        );
        assert_eq!(
            workout_extras(db, "i205"),
            (Some(1670.0), None),
            "a JSON integer becomes REAL and null stays NULL"
        );
        assert_eq!(
            workout_extras(db, "i206"),
            (None, None),
            "a JSON string is not a number"
        );
        assert_eq!(workout_extras(db, "w1"), (None, None));

        // No row was inserted for the body that has no workout.
        assert_eq!(count(db, "workouts"), 6);

        // The old columns are untouched.
        let duration: i64 = db
            .conn()
            .query_row(
                "SELECT duration_seconds FROM workouts WHERE id = 'i201'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(duration, 3540);
        assert_eq!(set_rows(db), empty_labels_converted());
    }

    #[test]
    fn v8_fills_workouts_that_have_a_stored_body() {
        let db = v7_db_with_workouts();
        run(&db).unwrap();
        assert_v8_backfilled(&db);
    }

    #[test]
    fn v8_runs_with_no_stored_bodies() {
        let db = v7_db_with_workouts();
        db.conn().execute("DELETE FROM raw_responses", []).unwrap();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        for id in ["w1", "i201", "i202", "i203", "i205", "i206"] {
            assert_eq!(workout_extras(&db, id), (None, None), "{id}");
        }
    }

    #[test]
    fn v8_can_run_twice() {
        let db = v7_db_with_workouts();
        migrate_v8(&db).unwrap();
        migrate_v8(&db).unwrap();
        assert_v8_backfilled(&db);
    }

    /// A database as pulse 0.9 left it: schema version 8, workouts without `hr_zone_seconds`, and
    /// stored Intervals.icu activity bodies with zone times for some of them.
    fn v8_db_with_zone_bodies() -> Database {
        let db = v7_db_with_workouts();
        migrate_v8(&db).unwrap();
        set_version(&db, 8).unwrap();
        db.conn()
            .execute_batch(
                r#"INSERT INTO workouts (id, start_time, duration_seconds, activity_type) VALUES
                 ('i211', '2026-10-16T07:00:00', 3540, 'Walk'),
                 ('i212', '2026-10-16T18:00:00', 3600, 'WeightTraining'),
                 ('i213', '2026-10-17T07:00:00', 1800, 'Run'),
                 ('i214', '2026-10-17T09:00:00', 1200, 'Walk'),
                 ('i215', '2026-10-17T10:00:00', 600, 'Walk');
                 INSERT INTO raw_responses (provider, endpoint, key, body) VALUES
                 ('intervals', 'activity', 'i211', '{"id":"i211","type":"Walk","icu_hr_zone_times":[491,158,150,110,28,0,0],"icu_hr_zones":[129,137,145,153,157,162,170]}'),
                 ('intervals', 'activity', 'i212', '{"id":"i212","type":"WeightTraining","icu_hr_zone_times":[0,0,0,0,0,0,0]}'),
                 ('intervals', 'activity', 'i213', '{"id":"i213","type":"Run","icu_hr_zone_times": [0, 0, 600, 1200, 30]}'),
                 ('intervals', 'activity', 'i214', '{"id":"i214","type":"Walk","icu_hr_zone_times":[491,null,0]}'),
                 ('intervals', 'activity', 'i216', '{"id":"i216","type":"Walk","icu_hr_zone_times":[1,2,3]}'),
                 ('garmin', 'activity', 'i215', '{"icu_hr_zone_times":[1,2,3]}');"#,
            )
            .unwrap();
        db
    }

    /// The stored `hr_zone_seconds` text of one workout.
    fn zone_text(db: &Database, id: &str) -> Option<String> {
        db.conn()
            .query_row(
                "SELECT hr_zone_seconds FROM workouts WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn v9_adds_the_zone_column() {
        let db = v8_db_with_zone_bodies();
        migrate_v9(&db).unwrap();

        assert_eq!(
            columns_of(&db, "workouts"),
            pairs(&[
                ("id", "TEXT"),
                ("start_time", "TEXT"),
                ("duration_seconds", "INTEGER"),
                ("activity_type", "TEXT"),
                ("calories", "INTEGER"),
                ("avg_hr", "INTEGER"),
                ("max_hr", "INTEGER"),
                ("source", "TEXT"),
                ("name", "TEXT"),
                ("distance_meters", "REAL"),
                ("avg_speed_mps", "REAL"),
                ("hr_zone_seconds", "TEXT"),
            ])
        );
    }

    /// Asserts the values the fixture's bodies produce, and that the other rows stayed empty.
    fn assert_v9_backfilled(db: &Database) {
        assert_eq!(
            zone_text(db, "i211").as_deref(),
            Some("[491,158,150,110,28,0,0]"),
            "a stored array is copied"
        );
        assert_eq!(zone_text(db, "i212"), None, "all zeros become NULL");
        assert_eq!(
            zone_text(db, "i213").as_deref(),
            Some("[0,0,600,1200,30]"),
            "five zones; whitespace dropped"
        );
        assert_eq!(
            zone_text(db, "i214"),
            None,
            "a null element voids the array"
        );
        assert_eq!(
            zone_text(db, "i215"),
            None,
            "the decoy under another provider was not used"
        );
        assert_eq!(
            zone_text(db, "i201"),
            None,
            "a body without the field gives NULL"
        );
        assert_eq!(zone_text(db, "w1"), None);

        // No row was inserted for the body that has no workout.
        assert_eq!(count(db, "workouts"), 11);

        // The old columns are untouched.
        assert_eq!(
            workout_extras(db, "i201"),
            (Some(5043.21), Some(1.417)),
            "the v8 columns keep their values"
        );
        assert_eq!(set_rows(db), empty_labels_converted());
    }

    #[test]
    fn v9_fills_workouts_that_have_a_stored_body() {
        let db = v8_db_with_zone_bodies();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        assert_v9_backfilled(&db);
    }

    #[test]
    fn v9_runs_with_no_stored_bodies() {
        let db = v8_db_with_zone_bodies();
        db.conn().execute("DELETE FROM raw_responses", []).unwrap();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        for id in [
            "w1", "i201", "i202", "i203", "i205", "i206", "i211", "i212", "i213", "i214", "i215",
        ] {
            assert_eq!(zone_text(&db, id), None, "{id}");
        }
    }

    #[test]
    fn v9_can_run_twice() {
        let db = v8_db_with_zone_bodies();
        migrate_v9(&db).unwrap();
        migrate_v9(&db).unwrap();
        assert_v9_backfilled(&db);
    }

    /// A database as pulse 0.10 left it: schema version 9, workouts without `training_load`, and
    /// stored Intervals.icu activity bodies with a load for some of them.
    fn v9_db_with_load_bodies() -> Database {
        let db = v8_db_with_zone_bodies();
        migrate_v9(&db).unwrap();
        set_version(&db, 9).unwrap();
        db.conn()
            .execute_batch(
                r#"INSERT INTO workouts (id, start_time, duration_seconds, activity_type) VALUES
                 ('i221', '2026-10-18T07:00:00', 3540, 'Walk'),
                 ('i222', '2026-10-18T18:00:00', 3600, 'WeightTraining'),
                 ('i223', '2026-10-19T07:00:00', 1800, 'Run'),
                 ('i224', '2026-10-19T09:00:00', 1200, 'Walk'),
                 ('i225', '2026-10-19T10:00:00', 600, 'Walk'),
                 ('i226', '2026-10-19T11:00:00', 600, 'Walk'),
                 ('i228', '2026-10-19T12:00:00', 600, 'Walk');
                 INSERT INTO raw_responses (provider, endpoint, key, body) VALUES
                 ('intervals', 'activity', 'i221', '{"id":"i221","type":"Walk","icu_training_load":43,"hr_load":43}'),
                 ('intervals', 'activity', 'i222', '{"id":"i222","type":"WeightTraining","icu_training_load":0}'),
                 ('intervals', 'activity', 'i223', '{"id":"i223","type":"Run","icu_training_load":57.9}'),
                 ('intervals', 'activity', 'i224', '{"id":"i224","type":"Walk","icu_training_load":null}'),
                 ('intervals', 'activity', 'i225', '{"id":"i225","type":"Walk","icu_training_load":"43"}'),
                 ('intervals', 'activity', 'i228', '{"id":"i228","type":"Walk","icu_training_load":3000000000}'),
                 ('intervals', 'activity', 'i227', '{"id":"i227","type":"Walk","icu_training_load":12}'),
                 ('garmin', 'activity', 'i226', '{"icu_training_load":99}');"#,
            )
            .unwrap();
        db
    }

    /// The stored `training_load` of one workout, read as `i64` so a broken bound fails an
    /// assertion instead of a conversion.
    fn load_of(db: &Database, id: &str) -> Option<i64> {
        db.conn()
            .query_row(
                "SELECT training_load FROM workouts WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn v10_adds_the_training_load_column() {
        let db = v9_db_with_load_bodies();
        migrate_v10(&db).unwrap();

        assert_eq!(
            columns_of(&db, "workouts"),
            pairs(&[
                ("id", "TEXT"),
                ("start_time", "TEXT"),
                ("duration_seconds", "INTEGER"),
                ("activity_type", "TEXT"),
                ("calories", "INTEGER"),
                ("avg_hr", "INTEGER"),
                ("max_hr", "INTEGER"),
                ("source", "TEXT"),
                ("name", "TEXT"),
                ("distance_meters", "REAL"),
                ("avg_speed_mps", "REAL"),
                ("hr_zone_seconds", "TEXT"),
                ("training_load", "INTEGER"),
            ])
        );
    }

    /// Asserts the values the fixture's bodies produce, and that the other rows stayed empty.
    fn assert_v10_backfilled(db: &Database) {
        assert_eq!(load_of(db, "i221"), Some(43), "a stored integer is copied");
        assert_eq!(load_of(db, "i223"), Some(57), "a fraction is truncated");
        assert_eq!(load_of(db, "i222"), None, "zero becomes NULL");
        assert_eq!(load_of(db, "i224"), None, "null gives NULL");
        assert_eq!(load_of(db, "i225"), None, "a string gives NULL");
        assert_eq!(load_of(db, "i228"), None, "beyond i32 gives NULL");
        assert_eq!(
            load_of(db, "i226"),
            None,
            "the decoy under another provider was not used"
        );
        assert_eq!(load_of(db, "i201"), None, "a body without the field");
        assert_eq!(load_of(db, "w1"), None);

        // No row was inserted for the body that has no workout.
        assert_eq!(count(db, "workouts"), 18);

        let not_integer: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM workouts WHERE typeof(training_load) NOT IN ('integer', 'null')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(not_integer, 0, "every stored load is an integer");

        // The old columns are untouched.
        assert_eq!(
            workout_extras(db, "i201"),
            (Some(5043.21), Some(1.417)),
            "the v8 columns keep their values"
        );
        assert_eq!(
            zone_text(db, "i211").as_deref(),
            Some("[491,158,150,110,28,0,0]"),
            "the v9 column keeps its value"
        );
        assert_eq!(set_rows(db), empty_labels_converted());
    }

    #[test]
    fn v10_fills_workouts_that_have_a_stored_body() {
        let db = v9_db_with_load_bodies();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        assert_v10_backfilled(&db);
    }

    #[test]
    fn v10_runs_with_no_stored_bodies() {
        let db = v9_db_with_load_bodies();
        db.conn().execute("DELETE FROM raw_responses", []).unwrap();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        let filled: i64 = db
            .conn()
            .query_row("SELECT COUNT(training_load) FROM workouts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(filled, 0);
    }

    #[test]
    fn v10_can_run_twice() {
        let db = v9_db_with_load_bodies();
        migrate_v10(&db).unwrap();
        migrate_v10(&db).unwrap();
        assert_v10_backfilled(&db);
    }

    /// A database as pulse 0.11 left it: schema version 10, no wellness table.
    fn v10_db() -> Database {
        let db = v9_db_with_load_bodies();
        migrate_v10(&db).unwrap();
        set_version(&db, 10).unwrap();
        db
    }

    #[test]
    fn v11_creates_the_wellness_table() {
        let db = v10_db();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);

        let mut stmt = db.conn().prepare("PRAGMA table_info(wellness)").unwrap();
        let columns: Vec<(String, String, bool, i32)> = stmt
            .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let expected: Vec<(String, String, bool, i32)> = [
            ("date", "TEXT", true, 1),
            ("weight_kg", "REAL", false, 0),
            ("body_fat_percent", "REAL", false, 0),
            ("ctl", "REAL", false, 0),
            ("atl", "REAL", false, 0),
            ("source", "TEXT", true, 0),
        ]
        .into_iter()
        .map(|(n, t, nn, pk)| (n.to_string(), t.to_string(), nn, pk))
        .collect();
        assert_eq!(columns, expected);

        let source_default: String = db
            .conn()
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('wellness') WHERE name = 'source'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(source_default, "'intervals'");
    }

    #[test]
    fn v11_leaves_the_other_tables_alone() {
        let db = v10_db();
        run(&db).unwrap();
        assert_v10_backfilled(&db);
    }

    #[test]
    fn v11_can_run_twice_and_keeps_rows() {
        let db = v10_db();
        migrate_v11(&db).unwrap();
        db.conn()
            .execute(
                "INSERT INTO wellness (date, weight_kg) VALUES ('2026-10-15', 78.4)",
                [],
            )
            .unwrap();
        migrate_v11(&db).unwrap();
        assert_eq!(count(&db, "wellness"), 1);
    }

    /// A database as pulse 0.12 left it: schema version 11, workouts without `hr_zone_max_bpm`,
    /// and stored Intervals.icu activity bodies with zone limits for some of them.
    fn v11_db_with_limit_bodies() -> Database {
        let db = v10_db();
        migrate_v11(&db).unwrap();
        set_version(&db, 11).unwrap();
        db.conn()
            .execute_batch(
                r#"INSERT INTO workouts (id, start_time, duration_seconds, activity_type) VALUES
                 ('i231', '2026-10-20T07:00:00', 3540, 'Walk'),
                 ('i232', '2026-10-20T18:00:00', 3600, 'WeightTraining'),
                 ('i233', '2026-10-21T07:00:00', 1800, 'Run'),
                 ('i234', '2026-10-21T09:00:00', 1200, 'Walk'),
                 ('i235', '2026-10-21T10:00:00', 600, 'Walk'),
                 ('i236', '2026-10-21T11:00:00', 600, 'Walk'),
                 ('i238', '2026-10-21T12:00:00', 600, 'Walk');
                 INSERT INTO raw_responses (provider, endpoint, key, body) VALUES
                 ('intervals', 'activity', 'i231', '{"id":"i231","type":"Walk","icu_hr_zone_times":[491,158,150,110,28,0,0],"icu_hr_zones":[123,137,143,153,157,162,170]}'),
                 ('intervals', 'activity', 'i232', '{"id":"i232","type":"WeightTraining","icu_hr_zones":[]}'),
                 ('intervals', 'activity', 'i233', '{"id":"i233","type":"Run","icu_hr_zones": [ 120 , 140 , 160 ]}'),
                 ('intervals', 'activity', 'i234', '{"id":"i234","type":"Walk","icu_hr_zones":[123,null,143]}'),
                 ('intervals', 'activity', 'i235', '{"id":"i235","type":"Walk","icu_hr_zones":[137,123]}'),
                 ('intervals', 'activity', 'i238', '{"id":"i238","type":"Walk","icu_hr_zones":[100,2147483648]}'),
                 ('intervals', 'activity', 'i237', '{"id":"i237","type":"Walk","icu_hr_zones":[1,2,3]}'),
                 ('garmin', 'activity', 'i236', '{"icu_hr_zones":[1,2,3]}');"#,
            )
            .unwrap();
        db
    }

    /// The stored `hr_zone_max_bpm` text of one workout.
    fn limit_text(db: &Database, id: &str) -> Option<String> {
        db.conn()
            .query_row(
                "SELECT hr_zone_max_bpm FROM workouts WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn v12_adds_the_zone_limit_column() {
        let db = v11_db_with_limit_bodies();
        migrate_v12(&db).unwrap();

        assert_eq!(
            columns_of(&db, "workouts"),
            pairs(&[
                ("id", "TEXT"),
                ("start_time", "TEXT"),
                ("duration_seconds", "INTEGER"),
                ("activity_type", "TEXT"),
                ("calories", "INTEGER"),
                ("avg_hr", "INTEGER"),
                ("max_hr", "INTEGER"),
                ("source", "TEXT"),
                ("name", "TEXT"),
                ("distance_meters", "REAL"),
                ("avg_speed_mps", "REAL"),
                ("hr_zone_seconds", "TEXT"),
                ("training_load", "INTEGER"),
                ("hr_zone_max_bpm", "TEXT"),
            ])
        );
    }

    /// Asserts the values the fixture's bodies produce, and that the other rows stayed empty.
    fn assert_v12_backfilled(db: &Database) {
        assert_eq!(
            limit_text(db, "i231").as_deref(),
            Some("[123,137,143,153,157,162,170]"),
            "a stored array is copied"
        );
        assert_eq!(
            limit_text(db, "i211").as_deref(),
            Some("[129,137,145,153,157,162,170]"),
            "an older workout gets its limits too"
        );
        assert_eq!(
            limit_text(db, "i233").as_deref(),
            Some("[120,140,160]"),
            "three zones; whitespace dropped"
        );
        assert_eq!(limit_text(db, "i232"), None, "empty");
        assert_eq!(
            limit_text(db, "i234"),
            None,
            "a null element voids the list"
        );
        assert_eq!(limit_text(db, "i235"), None, "not rising");
        assert_eq!(limit_text(db, "i238"), None, "beyond i32");
        assert_eq!(
            limit_text(db, "i236"),
            None,
            "the decoy under another provider was not used"
        );
        assert_eq!(limit_text(db, "i201"), None, "a body without the field");
        assert_eq!(limit_text(db, "w1"), None);

        // No row was inserted for the body that has no workout.
        assert_eq!(count(db, "workouts"), 25);

        // The earlier columns are untouched.
        assert_eq!(zone_text(db, "i231"), None, "v12 fills only its own column");
        assert_eq!(
            zone_text(db, "i211").as_deref(),
            Some("[491,158,150,110,28,0,0]"),
            "the v9 column keeps its value"
        );
        assert_eq!(
            load_of(db, "i221"),
            Some(43),
            "the v10 column keeps its value"
        );
        assert_eq!(
            workout_extras(db, "i201"),
            (Some(5043.21), Some(1.417)),
            "the v8 columns keep their values"
        );
        assert_eq!(set_rows(db), empty_labels_converted());
    }

    #[test]
    fn v12_fills_workouts_that_have_a_stored_body() {
        let db = v11_db_with_limit_bodies();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        assert_v12_backfilled(&db);
    }

    #[test]
    fn v12_runs_with_no_stored_bodies() {
        let db = v11_db_with_limit_bodies();
        db.conn().execute("DELETE FROM raw_responses", []).unwrap();
        run(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), CURRENT_VERSION);
        let filled: i64 = db
            .conn()
            .query_row("SELECT COUNT(hr_zone_max_bpm) FROM workouts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(filled, 0);
    }

    #[test]
    fn v12_can_run_twice() {
        let db = v11_db_with_limit_bodies();
        migrate_v12(&db).unwrap();
        migrate_v12(&db).unwrap();
        assert_v12_backfilled(&db);
    }
}
