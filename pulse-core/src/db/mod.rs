//! SQLite database wrapper, migrations, and query functions.
//!
//! Open a database with [`Database::open`] (uses `~/.pulse/pulse.db`) or
//! [`Database::open_memory`] for tests. Migrations run automatically on open.

use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub mod migrations;
pub mod queries;

/// Handle to the local SQLite database.
///
/// Wraps a `rusqlite::Connection` and runs schema migrations on creation.
/// The database file lives at `~/.pulse/pulse.db` by default.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create the database at `~/.pulse/pulse.db`.
    ///
    /// Creates the directory if it does not exist, enables WAL mode and
    /// foreign-key enforcement, and runs any pending migrations.
    pub fn open() -> Result<Self> {
        let db_path = crate::config::config_dir().join("pulse.db");
        Self::open_at(db_path)
    }

    /// Open at a specific path (useful for testing).
    ///
    /// Creates the parent directory if it does not exist.
    pub fn open_at(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        migrations::run(&db).with_context(|| format!("Failed to open {}", path.display()))?;
        Ok(db)
    }

    /// Open an in-memory database (for testing).
    ///
    /// Runs migrations so the schema is fully set up. Data is discarded when
    /// the `Database` is dropped.
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        migrations::run(&db)?;
        Ok(db)
    }

    /// Return a reference to the underlying `rusqlite::Connection`.
    ///
    /// Prefer the typed functions in [`queries`] over using this directly.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_at_refuses_a_database_from_a_newer_pulse() {
        let dir = std::env::temp_dir().join(format!("pulse-newer-db-{}", std::process::id()));
        let path = dir.join("pulse.db");
        drop(Database::open_at(path.clone()).unwrap());
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE sync_state SET value = '999' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();

        let message = format!("{:#}", Database::open_at(path.clone()).err().unwrap());

        let stored: String = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT value FROM sync_state WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(message.contains("version 999"), "{message}");
        assert_eq!(stored, "999");
    }
}
