//! Machine-readable output for the data commands.
//!
//! Each command wraps the rows it queried in a [`Report`] and prints the text a [`Report`]
//! produces. The hand-formatted terminal tables stay in `main.rs`.

use anyhow::Context;
use pulse_health_core::models::{ExerciseSet, Workout};
use pulse_health_core::vitality::{PaceInfo, VitalityScore};
use serde_json::{Map, Value};

mod ison;

/// How a data command prints its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Aligned table for reading in a terminal
    Table,
    /// Pretty-printed JSON
    Json,
    /// TOON: field names declared once per table (compact input for LLMs)
    Toon,
    /// ISON: field names declared once per table, cells separated by spaces (compact input for LLMs)
    Ison,
}

/// Output flags shared by every data command.
#[derive(Debug, Clone, Copy, clap::Args)]
pub struct OutputArgs {
    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    format: OutputFormat,

    /// Output as JSON (same as --format json)
    #[arg(long, conflicts_with = "format")]
    json: bool,
}

impl OutputArgs {
    /// The format to print: `--json` selects JSON, otherwise the value of `--format`.
    pub fn format(self) -> OutputFormat {
        if self.json {
            OutputFormat::Json
        } else {
            self.format
        }
    }
}

/// One data command's result in the machine-readable shapes pulse prints.
pub trait Report {
    /// Pretty-printed JSON, byte-identical to what `--json` printed before `--format` existed.
    fn json(&self) -> anyhow::Result<String>;

    /// The compact view for an LLM to read, which feeds TOON and ISON: a JSON object whose keys
    /// are table names in output order. Each value is an array of flat objects with identical
    /// keys (possibly empty), one flat object, null, or a group: an object that maps a parent id
    /// to such an array. Fields
    /// keep the model's declaration order.
    ///
    /// Unlike [`Report::json`] this is not a lossless copy of the data. It may leave out
    /// database row ids and round computed floats to the precision the terminal table shows.
    fn tables(&self) -> anyhow::Result<Value>;
}

/// Rows of one flat model (`sleep`, `heart`, `recovery`, `activity`, `stress`).
pub struct Rows<'a, T> {
    /// Table name in TOON and ISON output; the command name, which is also the SQLite table name.
    pub table: &'static str,
    /// The rows, newest first, as queried.
    pub rows: &'a [T],
}

/// Workouts, newest first, each with its exercise sets in `set_order`.
pub struct WorkoutLog<'a> {
    /// One entry per workout, paired with its exercise sets.
    pub entries: &'a [(Workout, Vec<ExerciseSet>)],
}

/// Vitality scores, newest first, plus the pace-of-aging summary when there is one.
pub struct VitalityReport<'a> {
    /// Pace of aging; `None` until enough days are scored.
    pub pace: Option<&'a PaceInfo>,
    /// Daily scores, newest first.
    pub scores: &'a [VitalityScore],
}

impl<T: serde::Serialize> Report for Rows<'_, T> {
    fn json(&self) -> anyhow::Result<String> {
        // Serialized directly, never through `Value`, so keys stay in declaration order
        // whatever serde_json features are enabled.
        Ok(serde_json::to_string_pretty(self.rows)?)
    }

    fn tables(&self) -> anyhow::Result<Value> {
        let mut tables = Map::new();
        tables.insert(self.table.into(), serde_json::to_value(self.rows)?);
        Ok(Value::Object(tables))
    }
}

impl Report for WorkoutLog<'_> {
    fn json(&self) -> anyhow::Result<String> {
        let mut workouts: Vec<Value> = Vec::with_capacity(self.entries.len());
        for (workout, sets) in self.entries {
            let mut value = serde_json::to_value(workout)?;
            value["exercise_sets"] = serde_json::to_value(sets)?;
            workouts.push(value);
        }
        let mut combined = Value::Array(workouts);
        // Keeps `--json` keys alphabetical even when serde_json's `preserve_order` is on.
        combined.sort_all_objects();
        Ok(serde_json::to_string_pretty(&combined)?)
    }

    fn tables(&self) -> anyhow::Result<Value> {
        let workouts: Vec<&Workout> = self.entries.iter().map(|(w, _)| w).collect();
        // One table of sets per workout, keyed by the workout id, so the id is stated once
        // and not on every row.
        let mut sets_by_workout = Map::new();
        for (workout, sets) in self.entries {
            if sets.is_empty() {
                continue;
            }
            let mut rows = serde_json::to_value(sets)?;
            for row in rows.as_array_mut().into_iter().flatten() {
                if let Some(row) = row.as_object_mut() {
                    // The group key carries the workout id, and the row id means nothing
                    // outside the database.
                    row.shift_remove("workout_id");
                    row.shift_remove("id");
                }
            }
            sets_by_workout.insert(workout.id.clone(), rows);
        }
        let mut tables = Map::new();
        tables.insert("workouts".into(), serde_json::to_value(workouts)?);
        // Null, not an empty group: a bare `exercise_sets:` as the last line reads as cut off.
        let sets = if sets_by_workout.is_empty() {
            Value::Null
        } else {
            Value::Object(sets_by_workout)
        };
        tables.insert("exercise_sets".into(), sets);
        Ok(Value::Object(tables))
    }
}

impl VitalityReport<'_> {
    /// `{pace, scores}` at full precision, keys in declaration order.
    fn value(&self) -> anyhow::Result<Value> {
        let mut value = Map::new();
        value.insert("pace".into(), serde_json::to_value(self.pace)?);
        value.insert("scores".into(), serde_json::to_value(self.scores)?);
        Ok(Value::Object(value))
    }
}

impl Report for VitalityReport<'_> {
    fn json(&self) -> anyhow::Result<String> {
        let mut combined = self.value()?;
        // Keeps `--json` keys alphabetical even when serde_json's `preserve_order` is on.
        combined.sort_all_objects();
        Ok(serde_json::to_string_pretty(&combined)?)
    }

    fn tables(&self) -> anyhow::Result<Value> {
        let mut tables = self.value()?;
        // Same precision as the terminal table: two decimals for the pace multiplier, one for
        // everything else. The scores are results of divisions and the rest is noise.
        if let Some(pace) = tables.get_mut("pace").and_then(Value::as_object_mut) {
            for (field, value) in pace.iter_mut() {
                round_floats(value, if field == "multiplier" { 2 } else { 1 });
            }
        }
        if let Some(scores) = tables.get_mut("scores") {
            round_floats(scores, 1);
        }
        Ok(tables)
    }
}

/// Rounds every float inside `value` to `decimals` places, the way `{:.N}` prints it, so the
/// result shows the same digits as the terminal table. Integers and non-numbers are untouched.
fn round_floats(value: &mut Value, decimals: usize) {
    match value {
        Value::Number(number) if number.is_f64() => {
            let rounded = number
                .as_f64()
                .and_then(|x| format!("{x:.decimals$}").parse::<f64>().ok())
                // A small negative rounds to `-0.0`; print it as plain zero.
                .map(|r| if r == 0.0 { 0.0 } else { r })
                .and_then(serde_json::Number::from_f64);
            if let Some(rounded) = rounded {
                *number = rounded;
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| round_floats(item, decimals)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| round_floats(field, decimals)),
        _ => {}
    }
}

/// Renders `report` in `format`. `Ok(None)` means `Table`: the caller prints its own table.
pub fn render(format: OutputFormat, report: &impl Report) -> anyhow::Result<Option<String>> {
    match format {
        OutputFormat::Table => Ok(None),
        OutputFormat::Json => report.json().map(Some),
        OutputFormat::Toon => toon_format::encode_default(&report.tables()?)
            .context("Failed to encode TOON output")
            .map(Some),
        OutputFormat::Ison => ison::encode(&report.tables()?)
            .context("Failed to encode ISON output")
            .map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_health_core::models::{Activity, Sleep};
    use pulse_health_core::vitality::PaceTrend;

    fn sleep_rows() -> Vec<Sleep> {
        vec![
            Sleep {
                date: "2026-01-02".into(),
                total_seconds: 27000,
                deep_seconds: 6300,
                rem_seconds: 5400,
                light_seconds: 14400,
                awake_seconds: 900,
                sleep_score: Some(82),
                hrv_ms: Some(48.0),
                source: "garmin".into(),
            },
            Sleep {
                date: "2026-01-01".into(),
                total_seconds: 25200,
                deep_seconds: 5400,
                rem_seconds: 4800,
                light_seconds: 14100,
                awake_seconds: 900,
                sleep_score: None,
                hrv_ms: Some(51.5),
                source: "garmin".into(),
            },
        ]
    }

    fn workout_log() -> Vec<(Workout, Vec<ExerciseSet>)> {
        vec![
            (
                Workout {
                    id: "i101".into(),
                    name: Some("Push day, heavy".into()),
                    start_time: "2026-01-02T07:30:00".into(),
                    duration_seconds: 3600,
                    activity_type: "WeightTraining".into(),
                    calories: Some(410),
                    avg_hr: Some(118),
                    max_hr: Some(152),
                    source: "intervals".into(),
                },
                vec![
                    ExerciseSet {
                        id: Some(1),
                        workout_id: "i101".into(),
                        set_order: 1,
                        exercise_category: Some("chest".into()),
                        exercise_name: Some("bench press".into()),
                        repetitions: Some(8),
                        weight_kg: Some(80.0),
                    },
                    ExerciseSet {
                        id: Some(2),
                        workout_id: "i101".into(),
                        set_order: 2,
                        exercise_category: None,
                        exercise_name: Some("pull-up".into()),
                        repetitions: Some(10),
                        weight_kg: None,
                    },
                ],
            ),
            (
                Workout {
                    id: "21234567890".into(),
                    name: None,
                    start_time: "2026-01-01T18:05:00".into(),
                    duration_seconds: 1800,
                    activity_type: "Run".into(),
                    calories: None,
                    avg_hr: None,
                    max_hr: None,
                    source: "intervals".into(),
                },
                vec![],
            ),
            (
                Workout {
                    id: "i103".into(),
                    name: Some("Legs: \"5x5\" - deload".into()),
                    start_time: "2026-01-01T06:00:00".into(),
                    duration_seconds: 2400,
                    activity_type: "WeightTraining".into(),
                    calories: Some(300),
                    avg_hr: Some(110),
                    max_hr: Some(140),
                    source: "intervals".into(),
                },
                vec![ExerciseSet {
                    id: Some(3),
                    workout_id: "i103".into(),
                    set_order: 1,
                    exercise_category: Some("legs".into()),
                    exercise_name: Some("squat".into()),
                    repetitions: Some(5),
                    weight_kg: Some(102.5),
                }],
            ),
        ]
    }

    /// One workout whose sets Garmin left partly or wholly unlabelled.
    fn unlabelled_log() -> Vec<(Workout, Vec<ExerciseSet>)> {
        let set = |set_order, category: Option<&str>, name: Option<&str>| ExerciseSet {
            id: Some(i64::from(set_order)),
            workout_id: "i104".into(),
            set_order,
            exercise_category: category.map(Into::into),
            exercise_name: name.map(Into::into),
            repetitions: Some(12),
            weight_kg: None,
        };
        vec![(
            Workout {
                id: "i104".into(),
                name: None,
                start_time: "2026-01-03T07:00:00".into(),
                duration_seconds: 900,
                activity_type: "WeightTraining".into(),
                calories: None,
                avg_hr: None,
                max_hr: None,
                source: "intervals".into(),
            },
            vec![
                set(1, Some("Curl"), None),
                set(2, None, None),
                set(3, Some("Plank"), Some("Side Plank")),
            ],
        )]
    }

    fn vitality_scores() -> Vec<VitalityScore> {
        vec![
            VitalityScore {
                date: "2026-01-02".into(),
                total_score: 78.4,
                sleep_score: 90.20833333333333,
                recovery_score: 71.25,
                activity_score: 100.0,
                stress_score: 75.0,
                fitness_score: Some(64.3),
            },
            VitalityScore {
                date: "2026-01-01".into(),
                total_score: 70.0,
                sleep_score: 76.5,
                recovery_score: 50.0,
                activity_score: 62.0,
                stress_score: 75.0,
                fitness_score: None,
            },
        ]
    }

    fn pace() -> PaceInfo {
        PaceInfo {
            multiplier: 1.0526315789473684,
            trend: PaceTrend::Steady,
            vs_baseline: -3.5,
            seven_day_avg: 74.2,
            thirty_day_avg: 70.5,
        }
    }

    const SLEEP_JSON: &str = r#"[
  {
    "date": "2026-01-02",
    "total_seconds": 27000,
    "deep_seconds": 6300,
    "rem_seconds": 5400,
    "light_seconds": 14400,
    "awake_seconds": 900,
    "sleep_score": 82,
    "hrv_ms": 48.0,
    "source": "garmin"
  },
  {
    "date": "2026-01-01",
    "total_seconds": 25200,
    "deep_seconds": 5400,
    "rem_seconds": 4800,
    "light_seconds": 14100,
    "awake_seconds": 900,
    "sleep_score": null,
    "hrv_ms": 51.5,
    "source": "garmin"
  }
]"#;

    const WORKOUTS_JSON: &str = r#"[
  {
    "activity_type": "WeightTraining",
    "avg_hr": 118,
    "calories": 410,
    "duration_seconds": 3600,
    "exercise_sets": [
      {
        "exercise_category": "chest",
        "exercise_name": "bench press",
        "id": 1,
        "repetitions": 8,
        "set_order": 1,
        "weight_kg": 80.0,
        "workout_id": "i101"
      },
      {
        "exercise_category": null,
        "exercise_name": "pull-up",
        "id": 2,
        "repetitions": 10,
        "set_order": 2,
        "weight_kg": null,
        "workout_id": "i101"
      }
    ],
    "id": "i101",
    "max_hr": 152,
    "name": "Push day, heavy",
    "source": "intervals",
    "start_time": "2026-01-02T07:30:00"
  },
  {
    "activity_type": "Run",
    "avg_hr": null,
    "calories": null,
    "duration_seconds": 1800,
    "exercise_sets": [],
    "id": "21234567890",
    "max_hr": null,
    "name": null,
    "source": "intervals",
    "start_time": "2026-01-01T18:05:00"
  },
  {
    "activity_type": "WeightTraining",
    "avg_hr": 110,
    "calories": 300,
    "duration_seconds": 2400,
    "exercise_sets": [
      {
        "exercise_category": "legs",
        "exercise_name": "squat",
        "id": 3,
        "repetitions": 5,
        "set_order": 1,
        "weight_kg": 102.5,
        "workout_id": "i103"
      }
    ],
    "id": "i103",
    "max_hr": 140,
    "name": "Legs: \"5x5\" - deload",
    "source": "intervals",
    "start_time": "2026-01-01T06:00:00"
  }
]"#;

    const VITALITY_JSON: &str = r#"{
  "pace": {
    "multiplier": 1.0526315789473684,
    "seven_day_avg": 74.2,
    "thirty_day_avg": 70.5,
    "trend": "steady",
    "vs_baseline": -3.5
  },
  "scores": [
    {
      "activity_score": 100.0,
      "date": "2026-01-02",
      "fitness_score": 64.3,
      "recovery_score": 71.25,
      "sleep_score": 90.20833333333333,
      "stress_score": 75.0,
      "total_score": 78.4
    },
    {
      "activity_score": 62.0,
      "date": "2026-01-01",
      "fitness_score": null,
      "recovery_score": 50.0,
      "sleep_score": 76.5,
      "stress_score": 75.0,
      "total_score": 70.0
    }
  ]
}"#;

    const VITALITY_NO_PACE_JSON: &str = r#"{
  "pace": null,
  "scores": [
    {
      "activity_score": 100.0,
      "date": "2026-01-02",
      "fitness_score": 64.3,
      "recovery_score": 71.25,
      "sleep_score": 90.20833333333333,
      "stress_score": 75.0,
      "total_score": 78.4
    }
  ]
}"#;

    /// One `Activity` row per string; the string goes in `source`, the last column.
    fn quoting_rows() -> Vec<Activity> {
        [
            "a,b",
            "a: b",
            "say \"hi\"",
            "-lead",
            "#tag",
            "true",
            "null",
            "123",
            "1e5",
            "",
            " pad ",
            "line\nbreak",
            "back\\slash",
            "[x]",
            "caf\u{e9} \u{2014} ok",
            "plain text",
            "pull-up",
            "2026-01-02",
        ]
        .iter()
        .map(|s| Activity {
            date: "2026-01-01".into(),
            steps: None,
            active_minutes: None,
            floors: None,
            source: (*s).to_string(),
        })
        .collect()
    }

    const SLEEP_TOON: &str = r#"sleep[2]{date,total_seconds,deep_seconds,rem_seconds,light_seconds,awake_seconds,sleep_score,hrv_ms,source}:
  2026-01-02,27000,6300,5400,14400,900,82,48,garmin
  2026-01-01,25200,5400,4800,14100,900,null,51.5,garmin"#;

    const WORKOUTS_TOON: &str = r#"workouts[3]{id,name,start_time,duration_seconds,activity_type,calories,avg_hr,max_hr,source}:
  i101,"Push day, heavy","2026-01-02T07:30:00",3600,WeightTraining,410,118,152,intervals
  "21234567890",null,"2026-01-01T18:05:00",1800,Run,null,null,null,intervals
  i103,"Legs: \"5x5\" - deload","2026-01-01T06:00:00",2400,WeightTraining,300,110,140,intervals
exercise_sets:
  i101[2]{set_order,exercise_category,exercise_name,repetitions,weight_kg}:
    1,chest,bench press,8,80
    2,null,pull-up,10,null
  i103[1]{set_order,exercise_category,exercise_name,repetitions,weight_kg}:
    1,legs,squat,5,102.5"#;

    const WORKOUTS_NO_SETS_TOON: &str = r#"workouts[1]{id,name,start_time,duration_seconds,activity_type,calories,avg_hr,max_hr,source}:
  "21234567890",null,"2026-01-01T18:05:00",1800,Run,null,null,null,intervals
exercise_sets: null"#;

    const WORKOUTS_NUMERIC_ID_TOON: &str = r#"workouts[1]{id,name,start_time,duration_seconds,activity_type,calories,avg_hr,max_hr,source}:
  "21234567890",null,"2026-01-01T18:05:00",1800,Run,null,null,null,intervals
exercise_sets:
  "21234567890"[1]{set_order,exercise_category,exercise_name,repetitions,weight_kg}:
    1,chest,bench press,8,80"#;

    const WORKOUTS_UNLABELLED_TOON: &str = r#"workouts[1]{id,name,start_time,duration_seconds,activity_type,calories,avg_hr,max_hr,source}:
  i104,null,"2026-01-03T07:00:00",900,WeightTraining,null,null,null,intervals
exercise_sets:
  i104[3]{set_order,exercise_category,exercise_name,repetitions,weight_kg}:
    1,Curl,null,12,null
    2,null,null,12,null
    3,Plank,Side Plank,12,null"#;

    const WORKOUTS_UNLABELLED_JSON: &str = r#"[
  {
    "activity_type": "WeightTraining",
    "avg_hr": null,
    "calories": null,
    "duration_seconds": 900,
    "exercise_sets": [
      {
        "exercise_category": "Curl",
        "exercise_name": null,
        "id": 1,
        "repetitions": 12,
        "set_order": 1,
        "weight_kg": null,
        "workout_id": "i104"
      },
      {
        "exercise_category": null,
        "exercise_name": null,
        "id": 2,
        "repetitions": 12,
        "set_order": 2,
        "weight_kg": null,
        "workout_id": "i104"
      },
      {
        "exercise_category": "Plank",
        "exercise_name": "Side Plank",
        "id": 3,
        "repetitions": 12,
        "set_order": 3,
        "weight_kg": null,
        "workout_id": "i104"
      }
    ],
    "id": "i104",
    "max_hr": null,
    "name": null,
    "source": "intervals",
    "start_time": "2026-01-03T07:00:00"
  }
]"#;

    const VITALITY_TOON: &str = r#"pace:
  multiplier: 1.05
  trend: steady
  vs_baseline: -3.5
  seven_day_avg: 74.2
  thirty_day_avg: 70.5
scores[2]{date,total_score,sleep_score,recovery_score,activity_score,stress_score,fitness_score}:
  2026-01-02,78.4,90.2,71.2,100,75,64.3
  2026-01-01,70,76.5,50,62,75,null"#;

    const VITALITY_NO_PACE_TOON: &str = r#"pace: null
scores[1]{date,total_score,sleep_score,recovery_score,activity_score,stress_score,fitness_score}:
  2026-01-02,78.4,90.2,71.2,100,75,64.3"#;

    // `"#tag"` ends an `r#"..."#` literal, hence the extra hash.
    const QUOTING_TOON: &str = r##"activity[18]{date,steps,active_minutes,floors,source}:
  2026-01-01,null,null,null,"a,b"
  2026-01-01,null,null,null,"a: b"
  2026-01-01,null,null,null,"say \"hi\""
  2026-01-01,null,null,null,"-lead"
  2026-01-01,null,null,null,"#tag"
  2026-01-01,null,null,null,"true"
  2026-01-01,null,null,null,"null"
  2026-01-01,null,null,null,"123"
  2026-01-01,null,null,null,"1e5"
  2026-01-01,null,null,null,""
  2026-01-01,null,null,null," pad "
  2026-01-01,null,null,null,"line\nbreak"
  2026-01-01,null,null,null,"back\\slash"
  2026-01-01,null,null,null,"[x]"
  2026-01-01,null,null,null,café — ok
  2026-01-01,null,null,null,plain text
  2026-01-01,null,null,null,pull-up
  2026-01-01,null,null,null,2026-01-02"##;

    const SLEEP_ISON: &str = r#"table.sleep
date total_seconds deep_seconds rem_seconds light_seconds awake_seconds sleep_score hrv_ms source
2026-01-02 27000 6300 5400 14400 900 82 48.0 garmin
2026-01-01 25200 5400 4800 14100 900 null 51.5 garmin"#;

    const WORKOUTS_ISON: &str = r#"table.workouts
id name start_time duration_seconds activity_type calories avg_hr max_hr source
i101 "Push day, heavy" 2026-01-02T07:30:00 3600 WeightTraining 410 118 152 intervals
"21234567890" null 2026-01-01T18:05:00 1800 Run null null null intervals
i103 "Legs: \"5x5\" - deload" 2026-01-01T06:00:00 2400 WeightTraining 300 110 140 intervals

table.exercise_sets.i101
set_order exercise_category exercise_name repetitions weight_kg
1 chest "bench press" 8 80.0
2 null pull-up 10 null

table.exercise_sets.i103
set_order exercise_category exercise_name repetitions weight_kg
1 legs squat 5 102.5"#;

    const WORKOUTS_NO_SETS_ISON: &str = r#"table.workouts
id name start_time duration_seconds activity_type calories avg_hr max_hr source
"21234567890" null 2026-01-01T18:05:00 1800 Run null null null intervals"#;

    const WORKOUTS_NUMERIC_ID_ISON: &str = r#"table.workouts
id name start_time duration_seconds activity_type calories avg_hr max_hr source
"21234567890" null 2026-01-01T18:05:00 1800 Run null null null intervals

table.exercise_sets.21234567890
set_order exercise_category exercise_name repetitions weight_kg
1 chest "bench press" 8 80.0"#;

    const WORKOUTS_UNLABELLED_ISON: &str = r#"table.workouts
id name start_time duration_seconds activity_type calories avg_hr max_hr source
i104 null 2026-01-03T07:00:00 900 WeightTraining null null null intervals

table.exercise_sets.i104
set_order exercise_category exercise_name repetitions weight_kg
1 Curl null 12 null
2 null null 12 null
3 Plank "Side Plank" 12 null"#;

    const VITALITY_ISON: &str = r#"object.pace
multiplier trend vs_baseline seven_day_avg thirty_day_avg
1.05 steady -3.5 74.2 70.5

table.scores
date total_score sleep_score recovery_score activity_score stress_score fitness_score
2026-01-02 78.4 90.2 71.2 100.0 75.0 64.3
2026-01-01 70.0 76.5 50.0 62.0 75.0 null"#;

    const VITALITY_NO_PACE_ISON: &str = r#"table.scores
date total_score sleep_score recovery_score activity_score stress_score fitness_score
2026-01-02 78.4 90.2 71.2 100.0 75.0 64.3"#;

    // `"#tag"` ends an `r#"..."#` literal, hence the extra hash.
    const QUOTING_ISON: &str = r##"table.activity
date steps active_minutes floors source
2026-01-01 null null null a,b
2026-01-01 null null null "a: b"
2026-01-01 null null null "say \"hi\""
2026-01-01 null null null -lead
2026-01-01 null null null "#tag"
2026-01-01 null null null "true"
2026-01-01 null null null "null"
2026-01-01 null null null "123"
2026-01-01 null null null "1e5"
2026-01-01 null null null ""
2026-01-01 null null null " pad "
2026-01-01 null null null "line\nbreak"
2026-01-01 null null null "back\\slash"
2026-01-01 null null null [x]
2026-01-01 null null null "café — ok"
2026-01-01 null null null "plain text"
2026-01-01 null null null pull-up
2026-01-01 null null null 2026-01-02"##;

    fn toon(report: &impl Report) -> String {
        render(OutputFormat::Toon, report)
            .unwrap()
            .expect("toon always renders text")
    }

    #[test]
    fn rows_toon_is_one_named_table() {
        let rows = sleep_rows();
        let report = Rows {
            table: "sleep",
            rows: &rows,
        };
        assert_eq!(toon(&report), SLEEP_TOON);
    }

    #[test]
    fn rows_toon_empty() {
        let report = Rows::<Sleep> {
            table: "sleep",
            rows: &[],
        };
        assert_eq!(toon(&report), "sleep: []");
    }

    #[test]
    fn workouts_toon_groups_sets_by_workout() {
        let entries = workout_log();
        assert_eq!(toon(&WorkoutLog { entries: &entries }), WORKOUTS_TOON);
    }

    #[test]
    fn workouts_toon_quotes_a_numeric_workout_id_key() {
        let mut entries = workout_log();
        let set = entries[0].1.remove(0);
        entries[1].1.push(set);
        let report = WorkoutLog {
            entries: &entries[1..2],
        };
        assert_eq!(toon(&report), WORKOUTS_NUMERIC_ID_TOON);
    }

    #[test]
    fn workouts_toon_prints_null_for_a_missing_label() {
        let entries = unlabelled_log();
        assert_eq!(
            toon(&WorkoutLog { entries: &entries }),
            WORKOUTS_UNLABELLED_TOON
        );
    }

    #[test]
    fn workouts_toon_without_sets() {
        let entries = workout_log();
        let report = WorkoutLog {
            entries: &entries[1..2],
        };
        assert_eq!(toon(&report), WORKOUTS_NO_SETS_TOON);
    }

    #[test]
    fn workouts_toon_empty() {
        assert_eq!(
            toon(&WorkoutLog { entries: &[] }),
            "workouts: []\nexercise_sets: null"
        );
    }

    #[test]
    fn vitality_toon_prints_a_small_negative_as_zero() {
        let pace = PaceInfo {
            vs_baseline: -0.04,
            ..pace()
        };
        let report = VitalityReport {
            pace: Some(&pace),
            scores: &[],
        };
        assert!(toon(&report).contains("\n  vs_baseline: 0\n"));
    }

    #[test]
    fn vitality_toon_is_pace_block_and_scores_table() {
        let (pace, scores) = (pace(), vitality_scores());
        let report = VitalityReport {
            pace: Some(&pace),
            scores: &scores,
        };
        assert_eq!(toon(&report), VITALITY_TOON);
    }

    #[test]
    fn vitality_toon_without_pace() {
        let scores = vitality_scores();
        let report = VitalityReport {
            pace: None,
            scores: &scores[..1],
        };
        assert_eq!(toon(&report), VITALITY_NO_PACE_TOON);
    }

    #[test]
    fn vitality_toon_empty() {
        let report = VitalityReport {
            pace: None,
            scores: &[],
        };
        assert_eq!(toon(&report), "pace: null\nscores: []");
    }

    #[test]
    fn toon_quotes_only_what_the_spec_requires() {
        let rows = quoting_rows();
        let report = Rows {
            table: "activity",
            rows: &rows,
        };
        assert_eq!(toon(&report), QUOTING_TOON);
    }

    // The name keeps the helper apart from the `ison` module that `use super::*` brings in.
    fn to_ison(report: &impl Report) -> String {
        render(OutputFormat::Ison, report)
            .unwrap()
            .expect("ison always renders text")
    }

    #[test]
    fn rows_ison_is_one_table_block() {
        let rows = sleep_rows();
        let report = Rows {
            table: "sleep",
            rows: &rows,
        };
        assert_eq!(to_ison(&report), SLEEP_ISON);
    }

    #[test]
    fn rows_ison_empty_is_an_empty_document() {
        let report = Rows::<Sleep> {
            table: "sleep",
            rows: &[],
        };
        assert_eq!(to_ison(&report), "");
    }

    #[test]
    fn workouts_ison_is_a_block_per_workout_with_sets() {
        let entries = workout_log();
        assert_eq!(to_ison(&WorkoutLog { entries: &entries }), WORKOUTS_ISON);
    }

    #[test]
    fn workouts_ison_without_sets() {
        let entries = workout_log();
        let report = WorkoutLog {
            entries: &entries[1..2],
        };
        assert_eq!(to_ison(&report), WORKOUTS_NO_SETS_ISON);
    }

    #[test]
    fn workouts_ison_names_a_block_after_a_numeric_workout_id() {
        let mut entries = workout_log();
        let set = entries[0].1.remove(0);
        entries[1].1.push(set);
        let report = WorkoutLog {
            entries: &entries[1..2],
        };
        assert_eq!(to_ison(&report), WORKOUTS_NUMERIC_ID_ISON);
    }

    #[test]
    fn workouts_ison_prints_null_for_a_missing_label() {
        let entries = unlabelled_log();
        assert_eq!(
            to_ison(&WorkoutLog { entries: &entries }),
            WORKOUTS_UNLABELLED_ISON
        );
    }

    #[test]
    fn workouts_ison_empty_is_an_empty_document() {
        assert_eq!(to_ison(&WorkoutLog { entries: &[] }), "");
    }

    #[test]
    fn vitality_ison_is_pace_object_and_scores_table() {
        let (pace, scores) = (pace(), vitality_scores());
        let report = VitalityReport {
            pace: Some(&pace),
            scores: &scores,
        };
        assert_eq!(to_ison(&report), VITALITY_ISON);
    }

    #[test]
    fn vitality_ison_without_pace_leaves_the_block_out() {
        let scores = vitality_scores();
        let report = VitalityReport {
            pace: None,
            scores: &scores[..1],
        };
        assert_eq!(to_ison(&report), VITALITY_NO_PACE_ISON);
    }

    #[test]
    fn vitality_ison_empty_is_an_empty_document() {
        let report = VitalityReport {
            pace: None,
            scores: &[],
        };
        assert_eq!(to_ison(&report), "");
    }

    #[test]
    fn vitality_ison_prints_a_small_negative_as_zero() {
        let pace = PaceInfo {
            vs_baseline: -0.04,
            ..pace()
        };
        let report = VitalityReport {
            pace: Some(&pace),
            scores: &[],
        };
        assert_eq!(
            to_ison(&report),
            "object.pace\nmultiplier trend vs_baseline seven_day_avg thirty_day_avg\n1.05 steady 0.0 74.2 70.5"
        );
    }

    #[test]
    fn ison_quotes_only_what_the_spec_requires() {
        let rows = quoting_rows();
        let report = Rows {
            table: "activity",
            rows: &rows,
        };
        assert_eq!(to_ison(&report), QUOTING_ISON);
    }

    #[test]
    fn render_table_is_none() {
        let rows = sleep_rows();
        let text = render(
            OutputFormat::Table,
            &Rows {
                table: "sleep",
                rows: &rows,
            },
        )
        .unwrap();
        assert!(text.is_none());
    }

    #[test]
    fn render_json_is_report_json() {
        let rows = sleep_rows();
        let text = render(
            OutputFormat::Json,
            &Rows {
                table: "sleep",
                rows: &rows,
            },
        )
        .unwrap();
        assert_eq!(text.as_deref(), Some(SLEEP_JSON));
    }

    #[test]
    fn rows_json_keeps_declaration_order() {
        let rows = sleep_rows();
        assert_eq!(
            Rows {
                table: "sleep",
                rows: &rows
            }
            .json()
            .unwrap(),
            SLEEP_JSON
        );
    }

    #[test]
    fn rows_json_empty_is_empty_array() {
        assert_eq!(
            Rows::<Sleep> {
                table: "sleep",
                rows: &[]
            }
            .json()
            .unwrap(),
            "[]"
        );
    }

    #[test]
    fn workouts_json_nests_sets_with_sorted_keys() {
        let entries = workout_log();
        assert_eq!(
            WorkoutLog { entries: &entries }.json().unwrap(),
            WORKOUTS_JSON
        );
    }

    #[test]
    fn workouts_json_prints_null_for_a_missing_label() {
        let entries = unlabelled_log();
        assert_eq!(
            WorkoutLog { entries: &entries }.json().unwrap(),
            WORKOUTS_UNLABELLED_JSON
        );
    }

    #[test]
    fn workouts_json_empty_is_empty_array() {
        assert_eq!(WorkoutLog { entries: &[] }.json().unwrap(), "[]");
    }

    #[test]
    fn vitality_json_has_sorted_keys() {
        let (pace, scores) = (pace(), vitality_scores());
        let report = VitalityReport {
            pace: Some(&pace),
            scores: &scores,
        };
        assert_eq!(report.json().unwrap(), VITALITY_JSON);
    }

    #[test]
    fn vitality_json_without_pace() {
        let scores = vitality_scores();
        let report = VitalityReport {
            pace: None,
            scores: &scores[..1],
        };
        assert_eq!(report.json().unwrap(), VITALITY_NO_PACE_JSON);
    }

    #[test]
    fn vitality_json_empty() {
        let report = VitalityReport {
            pace: None,
            scores: &[],
        };
        assert_eq!(
            report.json().unwrap(),
            "{\n  \"pace\": null,\n  \"scores\": []\n}"
        );
    }
}
