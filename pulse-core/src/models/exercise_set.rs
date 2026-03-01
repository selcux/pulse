use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExerciseSet {
    pub id: Option<i64>,
    pub workout_id: String,
    pub set_order: i32,
    pub exercise_category: Option<String>,
    pub exercise_name: String,
    pub repetitions: Option<i32>,
    pub weight_kg: Option<f64>,
}
