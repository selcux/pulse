//! Machine-readable output for the data commands.
//!
//! Each command wraps the rows it queried in a [`Report`] and prints the text a [`Report`]
//! produces. The hand-formatted terminal tables stay in `main.rs`.

use anyhow::Context;
use pulse_health_core::models::{ExerciseSet, Workout};
use pulse_health_core::vitality::{PaceInfo, VitalityScore};
use serde_json::{Map, Value};

/// How a data command prints its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Aligned table for reading in a terminal
    Table,
    /// Pretty-printed JSON
    Json,
    /// TOON: field names declared once per table (compact input for LLMs)
    Toon,
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

    /// The same data as named flat tables: a JSON object whose keys are table names in output
    /// order. Each value is an array of flat objects with identical keys (possibly empty), one
    /// flat object, or null. Fields keep the model's declaration order.
    fn tables(&self) -> anyhow::Result<Value>;
}

/// Rows of one flat model (`sleep`, `heart`, `recovery`, `activity`, `stress`).
pub struct Rows<'a, T> {
    /// Table name in TOON output; the command name, which is also the SQLite table name.
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
        let sets: Vec<&ExerciseSet> = self.entries.iter().flat_map(|(_, s)| s).collect();
        let mut tables = Map::new();
        tables.insert("workouts".into(), serde_json::to_value(workouts)?);
        tables.insert("exercise_sets".into(), serde_json::to_value(sets)?);
        Ok(Value::Object(tables))
    }
}

impl Report for VitalityReport<'_> {
    fn json(&self) -> anyhow::Result<String> {
        let mut combined = self.tables()?;
        // Keeps `--json` keys alphabetical even when serde_json's `preserve_order` is on.
        combined.sort_all_objects();
        Ok(serde_json::to_string_pretty(&combined)?)
    }

    fn tables(&self) -> anyhow::Result<Value> {
        let mut tables = Map::new();
        tables.insert("pace".into(), serde_json::to_value(self.pace)?);
        tables.insert("scores".into(), serde_json::to_value(self.scores)?);
        Ok(Value::Object(tables))
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
                        exercise_name: "bench press".into(),
                        repetitions: Some(8),
                        weight_kg: Some(80.0),
                    },
                    ExerciseSet {
                        id: Some(2),
                        workout_id: "i101".into(),
                        set_order: 2,
                        exercise_category: None,
                        exercise_name: "pull-up".into(),
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
                    exercise_name: "squat".into(),
                    repetitions: Some(5),
                    weight_kg: Some(102.5),
                }],
            ),
        ]
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
exercise_sets[3]{id,workout_id,set_order,exercise_category,exercise_name,repetitions,weight_kg}:
  1,i101,1,chest,bench press,8,80
  2,i101,2,null,pull-up,10,null
  3,i103,1,legs,squat,5,102.5"#;

    const WORKOUTS_NO_SETS_TOON: &str = r#"workouts[1]{id,name,start_time,duration_seconds,activity_type,calories,avg_hr,max_hr,source}:
  "21234567890",null,"2026-01-01T18:05:00",1800,Run,null,null,null,intervals
exercise_sets: []"#;

    const VITALITY_TOON: &str = r#"pace:
  multiplier: 1.0526315789473684
  trend: steady
  vs_baseline: -3.5
  seven_day_avg: 74.2
  thirty_day_avg: 70.5
scores[2]{date,total_score,sleep_score,recovery_score,activity_score,stress_score,fitness_score}:
  2026-01-02,78.4,90.20833333333333,71.25,100,75,64.3
  2026-01-01,70,76.5,50,62,75,null"#;

    const VITALITY_NO_PACE_TOON: &str = r#"pace: null
scores[1]{date,total_score,sleep_score,recovery_score,activity_score,stress_score,fitness_score}:
  2026-01-02,78.4,90.20833333333333,71.25,100,75,64.3"#;

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
    fn workouts_toon_is_two_flat_tables() {
        let entries = workout_log();
        assert_eq!(toon(&WorkoutLog { entries: &entries }), WORKOUTS_TOON);
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
            "workouts: []\nexercise_sets: []"
        );
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
