//! Core library for the [pulse](https://github.com/selcux/pulse) health CLI.
//!
//! `pulse-core` handles syncing health metrics from connected providers
//! ([Garmin Connect](https://connect.garmin.com), [Intervals.icu](https://intervals.icu)),
//! persisting them to a local SQLite database, and computing a composite
//! **vitality score** from sleep, recovery, activity, stress, and fitness data.
//!
//! # Modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`config`] | Load and write `~/.pulse/config.toml` |
//! | [`db`] | SQLite database, migrations, and queries |
//! | [`models`] | Plain data structs for each health metric |
//! | [`providers`] | [`Provider`](providers::Provider) trait + Garmin/Intervals implementations |
//! | [`sync`] | Orchestrate syncing across all enabled providers |
//! | [`vitality`] | Compute and trend composite vitality scores |
//!
//! # Quick start
//!
//! ```no_run
//! use pulse_core::{config, db::Database, sync, vitality};
//!
//! let cfg = config::load_config().unwrap();
//! let db = Database::open().unwrap();
//! sync::sync_all(&db, &cfg, 7).unwrap();
//! let scores = vitality::calculate_vitality(&db, &cfg, 7).unwrap();
//! println!("Today's vitality: {:.1}", scores[0].total_score);
//! ```

pub mod config;
pub mod db;
pub mod models;
pub mod providers;
pub mod sync;
pub mod vitality;
