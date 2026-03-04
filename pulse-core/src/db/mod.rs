//! SQLite database wrapper, migrations, and query functions.
//!
//! Open a database with [`Database::open`] (uses `~/.pulse/pulse.db`) or
//! [`Database::open_memory`] for tests. Migrations run automatically on open.

use std::path::PathBuf;

use anyhow::Result;
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
        migrations::run(&db)?;
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
