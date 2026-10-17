//! Terminal tables of the data commands: one column list per model, the renderer, and the cell
//! helpers. `--wide` prints every column of a list; the default view prints the columns its
//! `default_view` names.

use pulse_health_core::models::{Activity, ExerciseSet, Heart, Recovery, Sleep, Stress, Workout};
use pulse_health_core::vitality::VitalityScore;

/// Separator between two cells of a line.
const SEPARATOR: &str = " \u{2502} ";

/// Which side of its column a cell is padded to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Text starts at the left edge; padding goes after it.
    Left,
    /// Text ends at the right edge; padding goes before it.
    Right,
}

/// One column of a terminal table.
pub struct Column<T> {
    /// The model field the column shows, as serde names it (`"start_time"`). Selects the column
    /// in [`Table::default_view`]; tests check a column list against the model's fields.
    pub field: &'static str,
    /// Header text.
    pub header: &'static str,
    /// Minimum width in characters. The column grows to fit its header and its longest cell.
    pub width: usize,
    /// Which side of the column the cell is padded to.
    pub align: Align,
    /// Cell text of one row; `-` for an absent value.
    pub cell: fn(&T) -> String,
}

impl<T> Column<T> {
    /// A left-aligned column.
    pub const fn left(
        field: &'static str,
        header: &'static str,
        width: usize,
        cell: fn(&T) -> String,
    ) -> Self {
        Self {
            field,
            header,
            width,
            align: Align::Left,
            cell,
        }
    }

    /// A right-aligned column.
    pub const fn right(
        field: &'static str,
        header: &'static str,
        width: usize,
        cell: fn(&T) -> String,
    ) -> Self {
        Self {
            field,
            header,
            width,
            align: Align::Right,
            cell,
        }
    }
}

// Manual impls: `#[derive(Clone, Copy)]` would require `T: Copy`. Every field is Copy for any T.
impl<T> Clone for Column<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Column<T> {}

/// The terminal table of one model.
pub struct Table<T: 'static> {
    /// Every column, in the model's field order. `--wide` prints all of them.
    pub columns: &'static [Column<T>],
    /// `field` of each column the default view prints, in print order.
    pub default_view: &'static [&'static str],
}

impl<T> Table<T> {
    /// All columns when `wide`, else the `default_view` columns in that order.
    ///
    /// # Panics
    ///
    /// Panics if `default_view` names a field with no column: a typo in a const, which the golden
    /// tests catch.
    pub fn view(&self, wide: bool) -> Vec<Column<T>> {
        if wide {
            return self.columns.to_vec();
        }
        self.default_view
            .iter()
            .map(|field| {
                *self
                    .columns
                    .iter()
                    .find(|c| c.field == *field)
                    .expect("default_view names a field that has a column")
            })
            .collect()
    }

    /// The table of `rows`: every column when `wide`, else the default view.
    pub fn render(&self, rows: &[T], wide: bool) -> String {
        render(&self.view(wide), rows)
    }
}

/// Header line, rule line, then one line per row, joined by `\n` with no trailing newline.
///
/// A column is as wide as its minimum width, its header and its longest cell, whichever is most.
pub fn render<'a, T: 'a>(columns: &[Column<T>], rows: impl IntoIterator<Item = &'a T>) -> String {
    let cells: Vec<Vec<String>> = rows
        .into_iter()
        .map(|row| columns.iter().map(|c| (c.cell)(row)).collect())
        .collect();

    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let longest = cells.iter().map(|r| r[i].chars().count()).max();
            c.width
                .max(c.header.chars().count())
                .max(longest.unwrap_or(0))
        })
        .collect();

    let line = |texts: Vec<&str>| -> String {
        let padded: Vec<String> = texts
            .iter()
            .zip(columns.iter().zip(&widths))
            .map(|(text, (c, w))| match c.align {
                Align::Left => format!("{text:<w$}"),
                Align::Right => format!("{text:>w$}"),
            })
            .collect();
        padded.join(SEPARATOR).trim_end().to_string()
    };

    // A segment spans its column plus the padding either side of a separator, so each `┼` sits
    // under the `│` of the lines above.
    let last = widths.len().saturating_sub(1);
    let rule = widths
        .iter()
        .enumerate()
        .map(|(i, w)| "\u{2500}".repeat(w + usize::from(i > 0) + usize::from(i < last)))
        .collect::<Vec<_>>()
        .join("\u{253c}");

    let mut lines = vec![line(columns.iter().map(|c| c.header).collect()), rule];
    lines.extend(
        cells
            .iter()
            .map(|row| line(row.iter().map(String::as_str).collect())),
    );
    lines.join("\n")
}

/// Sleep: the columns in `Sleep`'s field order.
pub const SLEEP: Table<Sleep> = Table {
    columns: &[
        Column::left("date", "Date", 12, |s| s.date.clone()),
        Column::right("total_seconds", "Total", 8, |s| {
            format_duration(s.total_seconds)
        }),
        Column::right("deep_seconds", "Deep", 8, |s| {
            format_duration(s.deep_seconds)
        }),
        Column::right("rem_seconds", "REM", 8, |s| format_duration(s.rem_seconds)),
        Column::right("light_seconds", "Light", 8, |s| {
            format_duration(s.light_seconds)
        }),
        Column::right("awake_seconds", "Awake", 7, |s| {
            format_duration(s.awake_seconds)
        }),
        Column::right("sleep_score", "Score", 7, |s| opt_i32(s.sleep_score)),
        Column::right("hrv_ms", "HRV", 6, |s| opt_f64(s.hrv_ms, 1)),
        Column::right("start_time", "Bed", 5, |s| {
            clock_time(s.start_time.as_deref())
        }),
        Column::right("end_time", "Wake", 5, |s| clock_time(s.end_time.as_deref())),
        Column::right("spo2_avg", "SpO2", 5, |s| opt_f64(s.spo2_avg, 1)),
        Column::right("respiration_avg", "Resp", 5, |s| {
            opt_f64(s.respiration_avg, 1)
        }),
        Column::left("source", "Source", 6, |s| s.source.clone()),
    ],
    default_view: &[
        "date",
        "start_time",
        "end_time",
        "total_seconds",
        "deep_seconds",
        "rem_seconds",
        "light_seconds",
        "awake_seconds",
        "sleep_score",
        "hrv_ms",
    ],
};

/// Heart rate, HRV and VO2 max: the columns in `Heart`'s field order.
pub const HEART: Table<Heart> = Table {
    columns: &[
        Column::left("date", "Date", 12, |h| h.date.clone()),
        Column::right("resting_hr", "Resting HR", 10, |h| opt_i32(h.resting_hr)),
        Column::right("max_hr", "Max", 6, |h| opt_i32(h.max_hr)),
        Column::right("min_hr", "Min", 6, |h| opt_i32(h.min_hr)),
        Column::right("hrv_avg", "HRV avg", 8, |h| opt_f64(h.hrv_avg, 1)),
        Column::right("vo2_max", "VO2 max", 7, |h| opt_f64(h.vo2_max, 1)),
        Column::left("source", "Source", 6, |h| h.source.clone()),
    ],
    default_view: &[
        "date",
        "resting_hr",
        "max_hr",
        "min_hr",
        "hrv_avg",
        "vo2_max",
    ],
};

/// Body Battery: the columns in `Recovery`'s field order.
pub const RECOVERY: Table<Recovery> = Table {
    columns: &[
        Column::left("date", "Date", 12, |r| r.date.clone()),
        Column::right("body_battery_charged", "Chrgd", 7, |r| {
            opt_i32(r.body_battery_charged)
        }),
        Column::right("body_battery_drained", "Draind", 7, |r| {
            opt_i32(r.body_battery_drained)
        }),
        Column::right("body_battery_peak", "Peak", 7, |r| {
            opt_i32(r.body_battery_peak)
        }),
        Column::right("body_battery_low", "Low", 7, |r| {
            opt_i32(r.body_battery_low)
        }),
        Column::left("source", "Source", 6, |r| r.source.clone()),
    ],
    default_view: &[
        "date",
        "body_battery_peak",
        "body_battery_low",
        "body_battery_charged",
        "body_battery_drained",
    ],
};

/// Steps, calories and distance: the columns in `Activity`'s field order.
pub const ACTIVITY: Table<Activity> = Table {
    columns: &[
        Column::left("date", "Date", 12, |a| a.date.clone()),
        Column::right("steps", "Steps", 7, |a| opt_i32(a.steps)),
        Column::right("active_minutes", "Active min", 10, |a| {
            opt_i32(a.active_minutes)
        }),
        Column::right("floors", "Floors", 6, |a| opt_i32(a.floors)),
        Column::right("total_kcal", "Total kcal", 10, |a| opt_f64(a.total_kcal, 0)),
        Column::right("active_kcal", "Active kcal", 11, |a| {
            opt_f64(a.active_kcal, 0)
        }),
        Column::right("bmr_kcal", "BMR kcal", 8, |a| opt_f64(a.bmr_kcal, 0)),
        Column::right("distance_meters", "Dist km", 7, |a| {
            opt_f64(a.distance_meters.map(|m| m / 1000.0), 2)
        }),
        Column::left("source", "Source", 6, |a| a.source.clone()),
    ],
    default_view: &[
        "date",
        "steps",
        "active_minutes",
        "floors",
        "total_kcal",
        "distance_meters",
    ],
};

/// Stress levels and time at each level: the columns in `Stress`'s field order.
pub const STRESS: Table<Stress> = Table {
    columns: &[
        Column::left("date", "Date", 12, |s| s.date.clone()),
        Column::right("avg_stress", "Avg Stress", 10, |s| opt_i32(s.avg_stress)),
        Column::right("max_stress", "Max Stress", 10, |s| opt_i32(s.max_stress)),
        Column::right("rest_seconds", "Rest", 7, |s| opt_duration(s.rest_seconds)),
        Column::right("low_seconds", "Low", 7, |s| opt_duration(s.low_seconds)),
        Column::right("medium_seconds", "Med", 7, |s| {
            opt_duration(s.medium_seconds)
        }),
        Column::right("high_seconds", "High", 7, |s| opt_duration(s.high_seconds)),
        Column::left("source", "Source", 6, |s| s.source.clone()),
    ],
    default_view: &[
        "date",
        "avg_stress",
        "max_stress",
        "rest_seconds",
        "low_seconds",
        "medium_seconds",
        "high_seconds",
    ],
};

/// The vitality table. It has no wide view: every column is always shown.
pub const VITALITY_COLUMNS: &[Column<VitalityScore>] = &[
    Column::left("date", "Date", 12, |s| s.date.clone()),
    Column::right("total_score", "Total", 7, |s| {
        format!("{:.1}", s.total_score)
    }),
    Column::right("sleep_score", "Sleep", 7, |s| {
        format!("{:.1}", s.sleep_score)
    }),
    Column::right("recovery_score", "Recovery", 10, |s| {
        format!("{:.1}", s.recovery_score)
    }),
    Column::right("activity_score", "Activity", 10, |s| {
        format!("{:.1}", s.activity_score)
    }),
    Column::right("stress_score", "Stress", 8, |s| {
        format!("{:.1}", s.stress_score)
    }),
    Column::right("fitness_score", "Fitness", 9, |s| {
        opt_f64(s.fitness_score, 1)
    }),
];

/// Every workout column, in `Workout`'s field order (`pulse workouts --wide`).
pub const WORKOUT_COLUMNS: &[Column<Workout>] = &[
    Column::left("id", "ID", 2, |w| w.id.clone()),
    Column::left("name", "Name", 4, |w| opt_str(w.name.as_deref())),
    Column::left("start_time", "Start", 19, |w| w.start_time.clone()),
    Column::right("duration_seconds", "Duration", 8, |w| {
        format_duration(w.duration_seconds)
    }),
    Column::left("activity_type", "Type", 4, |w| w.activity_type.clone()),
    Column::right("calories", "kcal", 4, |w| opt_i32(w.calories)),
    Column::right("avg_hr", "Avg HR", 6, |w| opt_i32(w.avg_hr)),
    Column::right("max_hr", "Max HR", 6, |w| opt_i32(w.max_hr)),
    Column::left("source", "Source", 6, |w| w.source.clone()),
];

/// Every exercise set column except the database row id `id`, in `ExerciseSet`'s field order.
pub const EXERCISE_SET_COLUMNS: &[Column<ExerciseSet>] = &[
    Column::left("workout_id", "Workout", 7, |s| s.workout_id.clone()),
    Column::right("set_order", "Set", 3, |s| s.set_order.to_string()),
    Column::left("exercise_category", "Category", 8, |s| {
        opt_str(s.exercise_category.as_deref())
    }),
    Column::left("exercise_name", "Exercise", 8, |s| {
        opt_str(s.exercise_name.as_deref())
    }),
    Column::right("repetitions", "Reps", 4, |s| opt_i32(s.repetitions)),
    Column::right("weight_kg", "Weight", 6, |s| weight_or_bw(s.weight_kg)),
];

/// `pulse workouts --wide`: the workouts table, then, when any workout has sets, an empty line and
/// the exercise sets table (sets in the order given: newest workout first, then `set_order`).
pub fn workout_log_wide(entries: &[(Workout, Vec<ExerciseSet>)]) -> String {
    let mut text = render(WORKOUT_COLUMNS, entries.iter().map(|(w, _)| w));
    let mut sets = entries.iter().flat_map(|(_, sets)| sets).peekable();
    if sets.peek().is_some() {
        text.push_str("\n\n");
        text.push_str(&render(EXERCISE_SET_COLUMNS, sets));
    }
    text
}

/// `1h 05m`, or `30m` under an hour.
pub fn format_duration(seconds: i64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{}h {:02}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

/// `HH:MM` of a stored `YYYY-MM-DD HH:MM:SS` time; `-` when absent or too short.
pub fn clock_time(val: Option<&str>) -> String {
    val.and_then(|s| s.get(11..16)).unwrap_or("-").to_string()
}

/// `format_duration` of a value in seconds; `-` when absent.
pub fn opt_duration(val: Option<i64>) -> String {
    val.map_or_else(|| "-".into(), format_duration)
}

/// The number, or `-` when absent.
pub fn opt_i32(val: Option<i32>) -> String {
    match val {
        Some(v) => v.to_string(),
        None => "-".into(),
    }
}

/// The number with `decimals` decimals, or `-` when absent.
pub fn opt_f64(val: Option<f64>, decimals: usize) -> String {
    match val {
        Some(v) => format!("{v:.decimals$}"),
        None => "-".into(),
    }
}

/// The text, or `-` when absent.
pub fn opt_str(val: Option<&str>) -> String {
    val.unwrap_or("-").to_string()
}

/// `62.5kg` (one decimal), or `BW` (bodyweight) when absent: the workout log's weight text.
pub fn weight_or_bw(val: Option<f64>) -> String {
    match val {
        Some(w) => format!("{w:.1}kg"),
        None => "BW".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Item {
        name: &'static str,
        n: i32,
    }

    fn items() -> Vec<Item> {
        vec![
            Item { name: "ab", n: 7 },
            Item {
                name: "abcdef",
                n: 12,
            },
        ]
    }

    fn name_column(width: usize) -> Column<Item> {
        Column::left("name", "Name", width, |i| i.name.to_string())
    }

    fn n_column(width: usize) -> Column<Item> {
        Column::right("n", "N", width, |i| i.n.to_string())
    }

    fn lines(expected: &[&str]) -> String {
        expected.join("\n")
    }

    #[test]
    fn render_pads_cells_and_derives_the_rule() {
        assert_eq!(
            render(&[name_column(3), n_column(1)], &items()),
            lines(&["Name   │  N", "───────┼───", "ab     │  7", "abcdef │ 12",])
        );
    }

    #[test]
    fn render_does_not_pad_a_left_aligned_last_column() {
        assert_eq!(
            render(&[n_column(1), name_column(3)], &items()),
            lines(&[" N │ Name", "───┼───────", " 7 │ ab", "12 │ abcdef",])
        );
    }

    #[test]
    fn render_without_rows_prints_header_and_rule() {
        assert_eq!(
            render(&[name_column(3), n_column(1)], &[]),
            lines(&["Name │ N", "─────┼──"])
        );
    }

    #[test]
    fn rule_crosses_sit_under_the_separators() {
        let tables = [
            SLEEP.render(&sleep_rows(), false),
            SLEEP.render(&sleep_rows(), true),
            HEART.render(&heart_rows(), false),
            HEART.render(&heart_rows(), true),
            RECOVERY.render(&recovery_rows(), false),
            RECOVERY.render(&recovery_rows(), true),
            ACTIVITY.render(&activity_rows(), false),
            ACTIVITY.render(&activity_rows(), true),
            STRESS.render(&stress_rows(), false),
            STRESS.render(&stress_rows(), true),
            render(VITALITY_COLUMNS, &vitality_rows()),
            render(&[name_column(3), n_column(1)], &items()),
            render(&[n_column(1), name_column(3)], &items()),
            workout_log_wide(&workout_log()),
        ];
        let positions = |line: &str, mark: char| -> Vec<usize> {
            line.chars()
                .enumerate()
                .filter(|(_, c)| *c == mark)
                .map(|(i, _)| i)
                .collect()
        };
        for table in tables {
            let mut it = table.lines();
            let header = it.next().expect("a table has a header");
            let rule = it.next().expect("a table has a rule");
            assert_eq!(positions(rule, '┼'), positions(header, '│'), "{table}");
        }
    }

    #[test]
    fn clock_time_shows_hours_and_minutes() {
        assert_eq!(clock_time(Some("2026-10-15 02:22:10")), "02:22");
        assert_eq!(clock_time(None), "-");
        assert_eq!(clock_time(Some("02:22")), "-");
    }

    #[test]
    fn opt_duration_formats_or_dashes() {
        assert_eq!(opt_duration(Some(44340)), "12h 19m");
        assert_eq!(opt_duration(Some(420)), "7m");
        assert_eq!(opt_duration(None), "-");
    }

    #[test]
    fn opt_str_shows_the_text_or_a_dash() {
        assert_eq!(opt_str(Some("Run")), "Run");
        assert_eq!(opt_str(None), "-");
    }

    #[test]
    fn weight_or_bw_shows_kilograms_or_bodyweight() {
        assert_eq!(weight_or_bw(Some(62.5)), "62.5kg");
        assert_eq!(weight_or_bw(Some(80.0)), "80.0kg");
        assert_eq!(weight_or_bw(None), "BW");
    }

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
                start_time: Some("2026-01-01 23:10:00".into()),
                end_time: Some("2026-01-02 06:55:00".into()),
                spo2_avg: Some(95.0),
                respiration_avg: Some(14.5),
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
                hrv_ms: None,
                start_time: None,
                end_time: None,
                spo2_avg: None,
                respiration_avg: None,
                source: "garmin".into(),
            },
        ]
    }

    fn heart_rows() -> Vec<Heart> {
        vec![
            Heart {
                date: "2026-01-02".into(),
                resting_hr: Some(52),
                max_hr: Some(148),
                min_hr: Some(45),
                hrv_avg: Some(48.3),
                vo2_max: Some(47.6),
                source: "garmin".into(),
            },
            Heart {
                date: "2026-01-01".into(),
                resting_hr: None,
                max_hr: None,
                min_hr: None,
                hrv_avg: None,
                vo2_max: None,
                source: "garmin".into(),
            },
        ]
    }

    fn recovery_rows() -> Vec<Recovery> {
        vec![
            Recovery {
                date: "2026-01-02".into(),
                body_battery_charged: Some(62),
                body_battery_drained: Some(55),
                body_battery_peak: Some(88),
                body_battery_low: Some(26),
                source: "garmin".into(),
            },
            Recovery {
                date: "2026-01-01".into(),
                body_battery_charged: None,
                body_battery_drained: None,
                body_battery_peak: None,
                body_battery_low: None,
                source: "garmin".into(),
            },
        ]
    }

    fn activity_rows() -> Vec<Activity> {
        vec![
            Activity {
                date: "2026-01-02".into(),
                steps: Some(11234),
                active_minutes: Some(54),
                floors: Some(12),
                total_kcal: Some(2650.4),
                active_kcal: Some(720.6),
                bmr_kcal: Some(1929.8),
                distance_meters: Some(8765.4),
                source: "garmin".into(),
            },
            Activity {
                date: "2026-01-01".into(),
                steps: None,
                active_minutes: None,
                floors: None,
                total_kcal: None,
                active_kcal: None,
                bmr_kcal: None,
                distance_meters: None,
                source: "garmin".into(),
            },
        ]
    }

    fn stress_rows() -> Vec<Stress> {
        vec![
            Stress {
                date: "2026-01-02".into(),
                avg_stress: Some(28),
                max_stress: Some(87),
                rest_seconds: Some(44340),
                low_seconds: Some(18000),
                medium_seconds: Some(5400),
                high_seconds: Some(420),
                source: "garmin".into(),
            },
            Stress {
                date: "2026-01-01".into(),
                avg_stress: None,
                max_stress: None,
                rest_seconds: None,
                low_seconds: None,
                medium_seconds: None,
                high_seconds: None,
                source: "garmin".into(),
            },
        ]
    }

    fn vitality_rows() -> Vec<VitalityScore> {
        vec![
            VitalityScore {
                date: "2026-01-02".into(),
                total_score: 78.34,
                sleep_score: 81.27,
                recovery_score: 70.0,
                activity_score: 92.46,
                stress_score: 64.81,
                fitness_score: Some(66.12),
            },
            VitalityScore {
                date: "2026-01-01".into(),
                total_score: 61.0,
                sleep_score: 55.56,
                recovery_score: 48.9,
                activity_score: 100.0,
                stress_score: 40.04,
                fitness_score: None,
            },
        ]
    }

    fn workout_log() -> Vec<(Workout, Vec<ExerciseSet>)> {
        vec![
            (
                Workout {
                    id: "i101".into(),
                    name: Some("Upper Body".into()),
                    start_time: "2026-01-02T07:30:00".into(),
                    duration_seconds: 3900,
                    activity_type: "WeightTraining".into(),
                    calories: Some(320),
                    avg_hr: Some(112),
                    max_hr: Some(151),
                    source: "intervals".into(),
                },
                vec![
                    ExerciseSet {
                        id: Some(1),
                        workout_id: "i101".into(),
                        set_order: 1,
                        exercise_category: Some("Bench Press".into()),
                        exercise_name: Some("Barbell Bench Press".into()),
                        repetitions: Some(8),
                        weight_kg: Some(62.5),
                    },
                    ExerciseSet {
                        id: Some(2),
                        workout_id: "i101".into(),
                        set_order: 2,
                        exercise_category: Some("Shrug".into()),
                        exercise_name: None,
                        repetitions: Some(10),
                        weight_kg: None,
                    },
                ],
            ),
            (
                Workout {
                    id: "i100".into(),
                    name: None,
                    start_time: "2026-01-01T18:00:00".into(),
                    duration_seconds: 1800,
                    activity_type: "Run".into(),
                    calories: None,
                    avg_hr: None,
                    max_hr: None,
                    source: "intervals".into(),
                },
                vec![],
            ),
        ]
    }

    #[test]
    fn sleep_default_view_is_unchanged() {
        assert_eq!(
            SLEEP.render(&sleep_rows(), false),
            lines(&[
                "Date         │   Bed │  Wake │    Total │     Deep │      REM │    Light │   Awake │   Score │    HRV",
                "─────────────┼───────┼───────┼──────────┼──────────┼──────────┼──────────┼─────────┼─────────┼───────",
                "2026-01-02   │ 23:10 │ 06:55 │   7h 30m │   1h 45m │   1h 30m │   4h 00m │     15m │      82 │   48.0",
                "2026-01-01   │     - │     - │   7h 00m │   1h 30m │   1h 20m │   3h 55m │     15m │       - │      -",
            ])
        );
    }

    #[test]
    fn heart_default_view_adds_vo2_max_after_hrv_avg() {
        assert_eq!(
            HEART.render(&heart_rows(), false),
            lines(&[
                "Date         │ Resting HR │    Max │    Min │  HRV avg │ VO2 max",
                "─────────────┼────────────┼────────┼────────┼──────────┼────────",
                "2026-01-02   │         52 │    148 │     45 │     48.3 │    47.6",
                "2026-01-01   │          - │      - │      - │        - │       -",
            ])
        );
    }

    #[test]
    fn recovery_default_view_is_unchanged() {
        assert_eq!(
            RECOVERY.render(&recovery_rows(), false),
            lines(&[
                "Date         │    Peak │     Low │   Chrgd │  Draind",
                "─────────────┼─────────┼─────────┼─────────┼────────",
                "2026-01-02   │      88 │      26 │      62 │      55",
                "2026-01-01   │       - │       - │       - │       -",
            ])
        );
    }

    #[test]
    fn activity_default_view_is_unchanged() {
        assert_eq!(
            ACTIVITY.render(&activity_rows(), false),
            lines(&[
                "Date         │   Steps │ Active min │ Floors │ Total kcal │ Dist km",
                "─────────────┼─────────┼────────────┼────────┼────────────┼────────",
                "2026-01-02   │   11234 │         54 │     12 │       2650 │    8.77",
                "2026-01-01   │       - │          - │      - │          - │       -",
            ])
        );
    }

    #[test]
    fn stress_default_view_is_unchanged() {
        assert_eq!(
            STRESS.render(&stress_rows(), false),
            lines(&[
                "Date         │ Avg Stress │ Max Stress │    Rest │     Low │     Med │    High",
                "─────────────┼────────────┼────────────┼─────────┼─────────┼─────────┼────────",
                "2026-01-02   │         28 │         87 │ 12h 19m │  5h 00m │  1h 30m │      7m",
                "2026-01-01   │          - │          - │       - │       - │       - │       -",
            ])
        );
    }

    #[test]
    fn vitality_table_is_unchanged() {
        assert_eq!(
            render(VITALITY_COLUMNS, &vitality_rows()),
            lines(&[
                "Date         │   Total │   Sleep │   Recovery │   Activity │   Stress │   Fitness",
                "─────────────┼─────────┼─────────┼────────────┼────────────┼──────────┼──────────",
                "2026-01-02   │    78.3 │    81.3 │       70.0 │       92.5 │     64.8 │      66.1",
                "2026-01-01   │    61.0 │    55.6 │       48.9 │      100.0 │     40.0 │         -",
            ])
        );
    }

    #[test]
    fn sleep_wide_view_shows_every_column() {
        assert_eq!(
            SLEEP.render(&sleep_rows(), true),
            lines(&[
                "Date         │    Total │     Deep │      REM │    Light │   Awake │   Score │    HRV │   Bed │  Wake │  SpO2 │  Resp │ Source",
                "─────────────┼──────────┼──────────┼──────────┼──────────┼─────────┼─────────┼────────┼───────┼───────┼───────┼───────┼───────",
                "2026-01-02   │   7h 30m │   1h 45m │   1h 30m │   4h 00m │     15m │      82 │   48.0 │ 23:10 │ 06:55 │  95.0 │  14.5 │ garmin",
                "2026-01-01   │   7h 00m │   1h 30m │   1h 20m │   3h 55m │     15m │       - │      - │     - │     - │     - │     - │ garmin",
            ])
        );
    }

    #[test]
    fn heart_wide_view_shows_every_column() {
        assert_eq!(
            HEART.render(&heart_rows(), true),
            lines(&[
                "Date         │ Resting HR │    Max │    Min │  HRV avg │ VO2 max │ Source",
                "─────────────┼────────────┼────────┼────────┼──────────┼─────────┼───────",
                "2026-01-02   │         52 │    148 │     45 │     48.3 │    47.6 │ garmin",
                "2026-01-01   │          - │      - │      - │        - │       - │ garmin",
            ])
        );
    }

    #[test]
    fn recovery_wide_view_shows_every_column() {
        assert_eq!(
            RECOVERY.render(&recovery_rows(), true),
            lines(&[
                "Date         │   Chrgd │  Draind │    Peak │     Low │ Source",
                "─────────────┼─────────┼─────────┼─────────┼─────────┼───────",
                "2026-01-02   │      62 │      55 │      88 │      26 │ garmin",
                "2026-01-01   │       - │       - │       - │       - │ garmin",
            ])
        );
    }

    #[test]
    fn activity_wide_view_shows_every_column() {
        assert_eq!(
            ACTIVITY.render(&activity_rows(), true),
            lines(&[
                "Date         │   Steps │ Active min │ Floors │ Total kcal │ Active kcal │ BMR kcal │ Dist km │ Source",
                "─────────────┼─────────┼────────────┼────────┼────────────┼─────────────┼──────────┼─────────┼───────",
                "2026-01-02   │   11234 │         54 │     12 │       2650 │         721 │     1930 │    8.77 │ garmin",
                "2026-01-01   │       - │          - │      - │          - │           - │        - │       - │ garmin",
            ])
        );
    }

    #[test]
    fn stress_wide_view_shows_every_column() {
        assert_eq!(
            STRESS.render(&stress_rows(), true),
            lines(&[
                "Date         │ Avg Stress │ Max Stress │    Rest │     Low │     Med │    High │ Source",
                "─────────────┼────────────┼────────────┼─────────┼─────────┼─────────┼─────────┼───────",
                "2026-01-02   │         28 │         87 │ 12h 19m │  5h 00m │  1h 30m │      7m │ garmin",
                "2026-01-01   │          - │          - │       - │       - │       - │       - │ garmin",
            ])
        );
    }

    #[test]
    fn a_cell_wider_than_its_column_widens_it() {
        let mut rows = activity_rows();
        rows.truncate(1);
        rows[0].steps = Some(123_456_789);
        assert_eq!(
            ACTIVITY.render(&rows, false),
            lines(&[
                "Date         │     Steps │ Active min │ Floors │ Total kcal │ Dist km",
                "─────────────┼───────────┼────────────┼────────┼────────────┼────────",
                "2026-01-02   │ 123456789 │         54 │     12 │       2650 │    8.77",
            ])
        );
    }

    #[test]
    fn workout_log_wide_prints_workouts_then_sets() {
        assert_eq!(
            workout_log_wide(&workout_log()),
            lines(&[
                "ID   │ Name       │ Start               │ Duration │ Type           │ kcal │ Avg HR │ Max HR │ Source",
                "─────┼────────────┼─────────────────────┼──────────┼────────────────┼──────┼────────┼────────┼──────────",
                "i101 │ Upper Body │ 2026-01-02T07:30:00 │   1h 05m │ WeightTraining │  320 │    112 │    151 │ intervals",
                "i100 │ -          │ 2026-01-01T18:00:00 │      30m │ Run            │    - │      - │      - │ intervals",
                "",
                "Workout │ Set │ Category    │ Exercise            │ Reps │ Weight",
                "────────┼─────┼─────────────┼─────────────────────┼──────┼───────",
                "i101    │   1 │ Bench Press │ Barbell Bench Press │    8 │ 62.5kg",
                "i101    │   2 │ Shrug       │ -                   │   10 │     BW",
            ])
        );
    }

    #[test]
    fn workout_log_wide_without_sets_prints_only_workouts() {
        let mut log = workout_log();
        log.remove(0);
        assert_eq!(
            workout_log_wide(&log),
            lines(&[
                "ID   │ Name │ Start               │ Duration │ Type │ kcal │ Avg HR │ Max HR │ Source",
                "─────┼──────┼─────────────────────┼──────────┼──────┼──────┼────────┼────────┼──────────",
                "i100 │ -    │ 2026-01-01T18:00:00 │      30m │ Run  │    - │      - │      - │ intervals",
            ])
        );
    }

    fn model_fields(row: &impl serde::Serialize) -> Vec<String> {
        serde_json::to_value(row)
            .expect("a model serializes")
            .as_object()
            .expect("a model serializes to an object")
            .keys()
            .cloned()
            .collect()
    }

    fn column_fields<T>(columns: &[Column<T>]) -> Vec<String> {
        columns.iter().map(|c| c.field.to_string()).collect()
    }

    #[test]
    fn wide_views_list_every_model_field_in_declaration_order() {
        assert_eq!(column_fields(SLEEP.columns), model_fields(&sleep_rows()[0]));
        assert_eq!(column_fields(HEART.columns), model_fields(&heart_rows()[0]));
        assert_eq!(
            column_fields(RECOVERY.columns),
            model_fields(&recovery_rows()[0])
        );
        assert_eq!(
            column_fields(ACTIVITY.columns),
            model_fields(&activity_rows()[0])
        );
        assert_eq!(
            column_fields(STRESS.columns),
            model_fields(&stress_rows()[0])
        );
        assert_eq!(
            column_fields(WORKOUT_COLUMNS),
            model_fields(&workout_log()[0].0)
        );
        // `id` is the database row id of a set; it means nothing outside the database.
        let mut set_fields = model_fields(&workout_log()[0].1[0]);
        set_fields.retain(|f| f != "id");
        assert_eq!(column_fields(EXERCISE_SET_COLUMNS), set_fields);
    }
}
