use anyhow::{Context, Result};

use super::Database;

const CURRENT_VERSION: i32 = 2;

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
