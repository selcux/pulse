//! Plain data structs for each health metric stored in the local database.
//!
//! All structs implement [`serde::Serialize`] and [`serde::Deserialize`] so
//! they can be serialised to JSON (via `pulse <cmd> --json`) and stored in
//! SQLite via the [`db::queries`](crate::db::queries) functions.

pub mod activity;
pub mod exercise_set;
pub mod heart;
pub mod recovery;
pub mod sleep;
pub mod stress;
pub mod workout;

pub use activity::Activity;
pub use exercise_set::ExerciseSet;
pub use heart::Heart;
pub use recovery::Recovery;
pub use sleep::Sleep;
pub use stress::Stress;
pub use workout::Workout;
