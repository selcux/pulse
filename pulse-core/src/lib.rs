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
//! | [`date_range`] | [`DateRange`](date_range::DateRange): the inclusive dates that syncs, queries and vitality take |
//! | [`db`] | SQLite database, migrations, and queries |
//! | [`models`] | Plain data structs for each health metric |
//! | [`providers`] | [`Provider`](providers::Provider) trait + Garmin/Intervals implementations |
//! | [`sync`] | Orchestrate syncing across enabled providers, and rebuilding from stored responses |
//! | [`vitality`] | Compute and trend composite vitality scores |
//!
//! # Quick start
//!
//! ```no_run
//! use std::num::NonZeroU32;
//!
//! use chrono::Local;
//! use pulse_health_core::{config, date_range::DateRange, db::Database, sync, vitality};
//!
//! let cfg = config::load_config().unwrap();
//! let db = Database::open().unwrap();
//! let today = Local::now().date_naive();
//! let week = DateRange::last_days(today, NonZeroU32::new(7).unwrap());
//! sync::sync_all(&db, &cfg, &week).unwrap();
//! let scores = vitality::calculate_vitality(&db, &cfg, &week, today).unwrap();
//! println!("Today's vitality: {:.1}", scores[0].total_score);
//! ```

pub mod config;
pub mod date_range;
pub mod db;
pub mod models;
pub mod providers;
pub mod sync;
pub mod vitality;
