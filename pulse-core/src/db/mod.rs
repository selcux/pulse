use std::path::PathBuf;

use anyhow::Result;
use rusqlite::Connection;

pub mod migrations;
pub mod queries;

pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create the database at ~/.pulse/pulse.db
    pub fn open() -> Result<Self> {
        let db_path = crate::config::config_dir().join("pulse.db");
        Self::open_at(db_path)
    }

    /// Open at a specific path (useful for testing).
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
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        migrations::run(&db)?;
        Ok(db)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
