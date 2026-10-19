mod output;
mod range;
mod table;

use chrono::NaiveDate;
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use output::{OutputArgs, OutputFormat, Rows, StoredOutputArgs, VitalityReport, WorkoutLog};
use pulse_health_core::config;
use pulse_health_core::date_range::DateRange;
use pulse_health_core::db::Database;
use pulse_health_core::db::queries;
use pulse_health_core::models::{ExerciseSet, Workout};
use pulse_health_core::sync;
use pulse_health_core::vitality;
use range::{RangeArgs, SyncRangeArgs};
use table::format_duration;

#[derive(Parser)]
#[command(
    name = "pulse",
    about = "Unified health data from Garmin Connect and Intervals.icu",
    version,
    after_help = "Get started:\n  pulse garmin-login -u you@example.com\n  pulse sync garmin --days 7\n  pulse vitality --days 7"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with Garmin Connect (SSO + OAuth)
    GarminLogin {
        /// Garmin Connect username/email
        #[arg(short, long)]
        username: Option<String>,
        /// MFA verification code (if prompted on first attempt)
        #[arg(long)]
        mfa: Option<String>,
    },
    /// Sync health data from providers [garmin, intervals]
    Sync {
        /// Provider to sync (omit for all enabled)
        provider: Option<String>,
        #[command(flatten)]
        range: SyncRangeArgs,
    },
    /// Refill the data tables from stored provider responses (no network)
    Rebuild,
    /// Show sleep duration, stages, scores, and bed and wake times
    Sleep {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show resting heart rate, min/max HR, HRV, and VO2 max
    Heart {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show body battery peak/low values
    Recovery {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show steps, active minutes, floors, calories, and distance
    Activity {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show stress levels and time at each stress level
    Stress {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show workouts with distance, pace, heart rate zones, training load and exercise sets
    Workouts {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show weight, body fat, fitness (CTL) and fatigue (ATL) from Intervals.icu
    Wellness {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show composite vitality score (sleep + recovery + activity + stress, + fitness when available)
    Vitality {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Create default config.toml
    Init,
}

impl Commands {
    /// Subcommand name and output flags of the seven commands that print stored rows; `None` for the others.
    fn stored_output(&self) -> Option<(&'static str, StoredOutputArgs)> {
        match self {
            Commands::Sleep { output, .. } => Some(("sleep", *output)),
            Commands::Heart { output, .. } => Some(("heart", *output)),
            Commands::Recovery { output, .. } => Some(("recovery", *output)),
            Commands::Activity { output, .. } => Some(("activity", *output)),
            Commands::Stress { output, .. } => Some(("stress", *output)),
            Commands::Workouts { output, .. } => Some(("workouts", *output)),
            Commands::Wellness { output, .. } => Some(("wellness", *output)),
            _ => None,
        }
    }

    /// Subcommand name and resolved date range of the nine commands with range flags (`sync` and the
    /// eight data commands); `None` for the others.
    fn date_range(&self, today: NaiveDate) -> Option<(&'static str, anyhow::Result<DateRange>)> {
        match self {
            Commands::Sync { range, .. } => Some(("sync", range.resolve(today))),
            Commands::Sleep { range, .. } => Some(("sleep", range.resolve(today))),
            Commands::Heart { range, .. } => Some(("heart", range.resolve(today))),
            Commands::Recovery { range, .. } => Some(("recovery", range.resolve(today))),
            Commands::Activity { range, .. } => Some(("activity", range.resolve(today))),
            Commands::Stress { range, .. } => Some(("stress", range.resolve(today))),
            Commands::Workouts { range, .. } => Some(("workouts", range.resolve(today))),
            Commands::Wellness { range, .. } => Some(("wellness", range.resolve(today))),
            Commands::Vitality { range, .. } => Some(("vitality", range.resolve(today))),
            _ => None,
        }
    }
}

/// Builds the clap error of the subcommand `name`, so the message carries that command's usage line.
fn subcommand_error(name: &str, kind: ErrorKind, message: impl std::fmt::Display) -> clap::Error {
    let mut cmd = Cli::command();
    cmd.build();
    let sub = cmd
        .find_subcommand_mut(name)
        .expect("callers name a subcommand of Cli");
    sub.error(kind, message)
}

/// Parses the command line, then runs the checks clap cannot express: `--wide` with a `--format`
/// other than `table` (`--json --wide` is rejected by clap itself, through `conflicts_with`), and
/// the date range flags against `today` (`range.rs`).
fn parse_cli<I, T>(args: I, today: NaiveDate) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;
    if let Some((name, output)) = cli.command.stored_output()
        && output.wide()
        && output.format() != OutputFormat::Table
    {
        let format_name = output
            .format()
            .to_possible_value()
            .map(|v| v.get_name().to_string())
            .unwrap_or_default();
        return Err(subcommand_error(
            name,
            ErrorKind::ArgumentConflict,
            format!("the argument '--wide' cannot be used with '--format {format_name}'"),
        ));
    }
    if let Some((name, Err(e))) = cli.command.date_range(today) {
        return Err(subcommand_error(name, ErrorKind::ValueValidation, e));
    }
    Ok(cli)
}

fn main() -> anyhow::Result<()> {
    let today = chrono::Local::now().date_naive();
    let cli = parse_cli(std::env::args_os(), today).unwrap_or_else(|e| e.exit());

    match cli.command {
        Commands::GarminLogin { username, mfa } => cmd_garmin_login(username, mfa),
        Commands::Sync { provider, range } => cmd_sync(provider, &range.resolve(today)?),
        Commands::Rebuild => cmd_rebuild(),
        Commands::Sleep { range, output } => {
            cmd_sleep(&range.resolve(today)?, output.format(), output.wide())
        }
        Commands::Heart { range, output } => {
            cmd_heart(&range.resolve(today)?, output.format(), output.wide())
        }
        Commands::Recovery { range, output } => {
            cmd_recovery(&range.resolve(today)?, output.format(), output.wide())
        }
        Commands::Activity { range, output } => {
            cmd_activity(&range.resolve(today)?, output.format(), output.wide())
        }
        Commands::Stress { range, output } => {
            cmd_stress(&range.resolve(today)?, output.format(), output.wide())
        }
        Commands::Workouts { range, output } => {
            cmd_workouts(&range.resolve(today)?, output.format(), output.wide())
        }
        Commands::Wellness { range, output } => {
            cmd_wellness(&range.resolve(today)?, output.format(), output.wide())
        }
        Commands::Vitality { range, output } => {
            cmd_vitality(&range.resolve(today)?, today, output.format())
        }
        Commands::Config { action } => cmd_config(action),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_sync(provider: Option<String>, range: &DateRange) -> anyhow::Result<()> {
    let cfg = config::load_config()?;
    let db = Database::open()?;

    // The first sync that stores a response says so once; no state is kept for it.
    let store_was_empty = cfg.storage.raw_responses && queries::count_raw_responses(&db)? == 0;

    match provider {
        Some(name) => {
            println!("Syncing {}...", name);
            match sync::sync_provider(&db, &cfg, &name, range) {
                Ok(report) => {
                    println!("  {} records synced", report.records_synced);
                    for err in &report.errors {
                        println!("  error: {}", err);
                    }
                }
                Err(e) => println!("  error: {}", e),
            }
        }
        None => {
            let report = sync::sync_all(&db, &cfg, range)?;
            for r in &report.reports {
                if r.errors.is_empty() {
                    println!(
                        "Syncing {}... {} records synced",
                        r.provider, r.records_synced
                    );
                } else {
                    println!("Syncing {}... error: {}", r.provider, r.errors.join("; "));
                }
            }
            if report.reports.is_empty() {
                println!("No providers enabled. Run `pulse config` to check your configuration.");
            } else {
                println!(
                    "\nSync complete: {} synced, {} error(s)",
                    report.total_synced, report.total_errors
                );
            }
        }
    }

    if store_was_empty && queries::count_raw_responses(&db)? > 0 {
        println!(
            "\nNote: pulse now keeps each provider response, so later versions can fill new columns without a re-sync.\n\
             Days synced before this version have none. To cover them, sync them once: pulse sync --days N"
        );
    }

    Ok(())
}

fn cmd_rebuild() -> anyhow::Result<()> {
    let db = Database::open()?;
    let report = sync::rebuild_all(&db)?;

    if report.total_synced == 0 && report.total_errors == 0 {
        println!("Nothing to rebuild: no stored provider responses.");
        println!("`pulse sync` stores them while `[storage] raw_responses` is on (the default).");
        println!("To cover older days, sync them once: pulse sync --days N");
        return Ok(());
    }

    for r in &report.reports {
        println!("Rebuilding {}... {} records", r.provider, r.records_synced);
        for err in &r.errors {
            println!("  error: {}", err);
        }
    }
    println!(
        "Rebuild complete: {} records, {} error(s)",
        report.total_synced, report.total_errors
    );

    Ok(())
}

fn cmd_sleep(range: &DateRange, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_sleep(&db, range)?;

    if let Some(text) = output::render(
        format,
        &Rows {
            table: "sleep",
            rows: &data,
        },
    )? {
        println!("{text}");
        return Ok(());
    }

    if data.is_empty() {
        println!("No sleep data found from {} to {}.", range.start, range.end);
        return Ok(());
    }

    println!("{}", table::SLEEP.render(&data, wide));

    Ok(())
}

fn cmd_heart(range: &DateRange, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_heart(&db, range)?;

    if let Some(text) = output::render(
        format,
        &Rows {
            table: "heart",
            rows: &data,
        },
    )? {
        println!("{text}");
        return Ok(());
    }

    if data.is_empty() {
        println!("No heart data found from {} to {}.", range.start, range.end);
        return Ok(());
    }

    println!("{}", table::HEART.render(&data, wide));

    Ok(())
}

fn cmd_recovery(range: &DateRange, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_recovery(&db, range)?;

    if let Some(text) = output::render(
        format,
        &Rows {
            table: "recovery",
            rows: &data,
        },
    )? {
        println!("{text}");
        return Ok(());
    }

    if data.is_empty() {
        println!(
            "No recovery data found from {} to {}.",
            range.start, range.end
        );
        return Ok(());
    }

    println!("{}", table::RECOVERY.render(&data, wide));

    Ok(())
}

fn cmd_activity(range: &DateRange, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_activity(&db, range)?;

    if let Some(text) = output::render(
        format,
        &Rows {
            table: "activity",
            rows: &data,
        },
    )? {
        println!("{text}");
        return Ok(());
    }

    if data.is_empty() {
        println!(
            "No activity data found from {} to {}.",
            range.start, range.end
        );
        return Ok(());
    }

    println!("{}", table::ACTIVITY.render(&data, wide));

    Ok(())
}

fn cmd_stress(range: &DateRange, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_stress(&db, range)?;

    if let Some(text) = output::render(
        format,
        &Rows {
            table: "stress",
            rows: &data,
        },
    )? {
        println!("{text}");
        return Ok(());
    }

    if data.is_empty() {
        println!(
            "No stress data found from {} to {}.",
            range.start, range.end
        );
        return Ok(());
    }

    println!("{}", table::STRESS.render(&data, wide));

    Ok(())
}

fn cmd_wellness(range: &DateRange, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_wellness(&db, range)?;

    if let Some(text) = output::render(
        format,
        &Rows {
            table: "wellness",
            rows: &data,
        },
    )? {
        println!("{text}");
        return Ok(());
    }

    if data.is_empty() {
        println!(
            "No wellness data found from {} to {}.",
            range.start, range.end
        );
        return Ok(());
    }

    println!("{}", table::WELLNESS.render(&data, wide));

    Ok(())
}

fn cmd_workouts(range: &DateRange, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let workouts = queries::query_workouts(&db, range)?;
    let mut entries: Vec<(Workout, Vec<ExerciseSet>)> = Vec::with_capacity(workouts.len());
    for w in workouts {
        let sets = queries::query_exercise_sets(&db, &w.id)?;
        entries.push((w, sets));
    }

    if let Some(text) = output::render(format, &WorkoutLog { entries: &entries })? {
        println!("{text}");
        return Ok(());
    }

    if entries.is_empty() {
        println!("No workouts found from {} to {}.", range.start, range.end);
        return Ok(());
    }

    if wide {
        println!("{}", table::workout_log_wide(&entries));
        return Ok(());
    }

    for (w, sets) in &entries {
        println!("{}", format_workout_line(w));
        if let Some(line) = format_zone_line(w) {
            println!("{line}");
        }

        for s in sets {
            println!("{}", format_exercise_set(s));
        }
        if !sets.is_empty() {
            println!();
        }
    }

    Ok(())
}

/// The workout log line: `[start] label (duration[, distance][, pace], kcal, HR[, load])`.
/// Distance, pace and load are left out when absent, so a strength workout reads as before.
fn format_workout_line(w: &Workout) -> String {
    let mut parts = vec![format_duration(w.duration_seconds)];
    if let Some(meters) = w.distance_meters {
        parts.push(format!("{:.2} km", meters / 1000.0));
    }
    if let Some(pace) = table::pace_per_km(w.avg_speed_mps) {
        parts.push(format!("{pace} /km"));
    }
    parts.push(match w.calories {
        Some(c) => format!("{}cal", c),
        None => "-".into(),
    });
    parts.push(match w.avg_hr {
        Some(h) => format!("avg HR {}", h),
        None => "no HR".into(),
    });
    if let Some(load) = w.training_load {
        parts.push(format!("load {load}"));
    }
    let label = match &w.name {
        Some(n) => format!("{} \u{2014} {}", n, w.activity_type),
        None => w.activity_type.clone(),
    };
    format!("[{}] {} ({})", w.start_time, label, parts.join(", "))
}

/// The log line under a workout with heart rate zone times: `  HR zones: Z1 ≤123: 8m · Z2 ≤137: 2m`,
/// without the tops when the workout has none or a different number of them. `None` when no zone
/// has time, so no line is printed.
fn format_zone_line(w: &Workout) -> Option<String> {
    let zones =
        table::format_hr_zones(w.hr_zone_seconds.as_deref()?, w.hr_zone_max_bpm.as_deref())?;
    Some(format!("  HR zones: {zones}"))
}

/// Name column text: the exercise name, else the category in parentheses so it
/// is not mistaken for the exact exercise, else `-`.
fn exercise_label(s: &ExerciseSet) -> String {
    match (&s.exercise_name, &s.exercise_category) {
        (Some(name), _) => name.clone(),
        (None, Some(category)) => format!("({category})"),
        (None, None) => "-".into(),
    }
}

fn format_exercise_set(s: &ExerciseSet) -> String {
    let reps = match s.repetitions {
        Some(r) => format!("{}", r),
        None => "-".into(),
    };
    format!(
        "  {:>2}. {:<20} {:>3} x {}",
        s.set_order,
        exercise_label(s),
        reps,
        table::weight_or_bw(s.weight_kg)
    )
}

fn cmd_vitality(
    range: &DateRange,
    today: chrono::NaiveDate,
    format: OutputFormat,
) -> anyhow::Result<()> {
    let cfg = config::load_config()?;
    let db = Database::open()?;
    let scores = vitality::calculate_vitality(&db, &cfg, range, today)?;
    let pace = vitality::calculate_pace(&db, &cfg, range.end, today)?;

    let report = VitalityReport {
        pace: pace.as_ref(),
        scores: &scores,
    };
    if let Some(text) = output::render(format, &report)? {
        println!("{text}");
        return Ok(());
    }

    if cfg.user.lean_body_mass_kg.is_none() {
        eprintln!(
            "Note: lean_body_mass_kg not set in ~/.pulse/config.toml — \
             fitness score uses VO2 Max only. Set it to improve accuracy."
        );
    }

    if let Some(ref p) = pace {
        let (emoji, label) = match p.trend {
            vitality::PaceTrend::Slowing => ("\u{1f7e2}", "Slowing"),
            vitality::PaceTrend::Steady => ("\u{1f7e1}", "Steady"),
            vitality::PaceTrend::Accelerating => ("\u{1f534}", "Accelerating"),
        };
        let sign = if p.vs_baseline >= 0.0 { "+" } else { "" };
        println!(
            "Pace of aging: {:.2}x {} {}   7d avg: {:.1} | 30d avg: {:.1} ({}{:.1} pts)",
            p.multiplier, emoji, label, p.seven_day_avg, p.thirty_day_avg, sign, p.vs_baseline
        );
        println!();
    }

    if scores.is_empty() {
        println!(
            "No vitality data from {} to {}. (Sleep data is required as anchor.)",
            range.start, range.end
        );
        return Ok(());
    }

    println!("{}", table::render(table::VITALITY_COLUMNS, &scores));
    if scores.iter().any(|s| !s.complete) {
        println!("{}", table::UNFINISHED_FOOTNOTE);
    }

    Ok(())
}

fn cmd_garmin_login(username: Option<String>, mfa: Option<String>) -> anyhow::Result<()> {
    use pulse_health_core::providers::garmin::GarminProvider;
    use pulse_health_core::providers::garmin::auth::LoginResult;

    // Resolve username: CLI arg > config > prompt
    let email = if let Some(u) = username {
        u
    } else {
        let cfg = config::load_config()?;
        if let Some(garmin_cfg) = &cfg.providers.garmin {
            if let Some(ref u) = garmin_cfg.username {
                println!("Using username from config: {}", u);
                u.clone()
            } else {
                prompt_input("Garmin email: ")?
            }
        } else {
            prompt_input("Garmin email: ")?
        }
    };

    let password = rpassword::prompt_password("Garmin password: ")?;

    let provider = GarminProvider::new();
    match provider.login(&email, &password, mfa.as_deref())? {
        LoginResult::Success => {
            println!("Login successful! Tokens saved to ~/.pulse/garmin/");
            println!("You can now run: pulse sync garmin");
        }
        LoginResult::MfaRequired => {
            println!("MFA required. Re-run with --mfa <code>:");
            println!("  pulse garmin-login -u {} --mfa <code>", email);
        }
    }

    Ok(())
}

fn prompt_input(prompt: &str) -> anyhow::Result<String> {
    use std::io::Write;
    print!("{}", prompt);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn cmd_config(action: Option<ConfigAction>) -> anyhow::Result<()> {
    match action {
        Some(ConfigAction::Init) => {
            let path = config::write_default_config()?;
            println!("Default config written to: {}", path.display());
        }
        None => {
            let cfg = config::load_config()?;
            let toml_str = toml::to_string_pretty(&cfg)?;
            println!("{}", toml_str);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA_COMMANDS: [&str; 8] = [
        "sleep", "heart", "recovery", "activity", "stress", "workouts", "wellness", "vitality",
    ];

    const STORED_COMMANDS: [&str; 7] = [
        "sleep", "heart", "recovery", "activity", "stress", "workouts", "wellness",
    ];

    const RANGE_COMMANDS: [&str; 9] = [
        "sync", "sleep", "heart", "recovery", "activity", "stress", "workouts", "wellness",
        "vitality",
    ];

    fn today() -> NaiveDate {
        date("2026-10-17")
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn range(start: &str, end: &str) -> DateRange {
        DateRange {
            start: date(start),
            end: date(end),
        }
    }

    fn parse_range(args: &[&str]) -> Result<DateRange, ErrorKind> {
        let cli = parse_cli(args, today()).map_err(|e| e.kind())?;
        let (_, range) = cli
            .command
            .date_range(today())
            .expect("not a range command");
        Ok(range.expect("parse_cli already resolved it"))
    }

    fn parse_format(args: &[&str]) -> Result<OutputFormat, ErrorKind> {
        let cli = parse_cli(args, today()).map_err(|e| e.kind())?;
        match cli.command {
            Commands::Vitality { output, .. } => Ok(output.format()),
            other => Ok(other
                .stored_output()
                .expect("not a data command")
                .1
                .format()),
        }
    }

    fn parse_wide(args: &[&str]) -> Result<bool, ErrorKind> {
        let cli = parse_cli(args, today()).map_err(|e| e.kind())?;
        Ok(cli
            .command
            .stored_output()
            .expect("not a stored row command")
            .1
            .wide())
    }

    fn exercise_set(name: Option<&str>, category: Option<&str>) -> ExerciseSet {
        ExerciseSet {
            id: Some(1),
            workout_id: "i101".into(),
            set_order: 2,
            exercise_category: category.map(Into::into),
            exercise_name: name.map(Into::into),
            repetitions: Some(8),
            weight_kg: Some(62.5),
        }
    }

    #[test]
    fn exercise_set_line_shows_the_name_over_the_category() {
        assert_eq!(
            format_exercise_set(&exercise_set(
                Some("Barbell Bench Press"),
                Some("Bench Press")
            )),
            "   2. Barbell Bench Press    8 x 62.5kg"
        );
    }

    #[test]
    fn exercise_set_line_falls_back_to_the_category_in_parentheses() {
        assert_eq!(
            format_exercise_set(&exercise_set(None, Some("Bench Press"))),
            "   2. (Bench Press)          8 x 62.5kg"
        );
    }

    #[test]
    fn exercise_set_line_shows_a_dash_without_name_or_category() {
        assert_eq!(
            format_exercise_set(&exercise_set(None, None)),
            "   2. -                      8 x 62.5kg"
        );
    }

    fn workout(distance: Option<f64>, speed: Option<f64>) -> Workout {
        Workout {
            id: "i201".into(),
            name: None,
            start_time: "2026-10-15T07:00:00".into(),
            duration_seconds: 3540,
            activity_type: "Walk".into(),
            calories: Some(287),
            avg_hr: Some(104),
            max_hr: None,
            distance_meters: distance,
            avg_speed_mps: speed,
            hr_zone_seconds: None,
            hr_zone_max_bpm: None,
            training_load: None,
            source: "intervals".into(),
        }
    }

    #[test]
    fn zone_line_lists_the_zones_with_time() {
        let w = Workout {
            hr_zone_seconds: Some(vec![491, 158, 150, 110, 28, 0, 0]),
            ..workout(None, None)
        };
        assert_eq!(
            format_zone_line(&w),
            Some(
                "  HR zones: Z1 8m \u{b7} Z2 2m \u{b7} Z3 2m \u{b7} Z4 1m \u{b7} Z5 <1m"
                    .to_string()
            )
        );
    }

    #[test]
    fn zone_line_labels_each_zone_with_its_top() {
        let w = Workout {
            hr_zone_seconds: Some(vec![491, 158, 150, 110, 28, 0, 0]),
            hr_zone_max_bpm: Some(vec![123, 137, 143, 153, 157, 162, 170]),
            ..workout(None, None)
        };
        assert_eq!(
            format_zone_line(&w),
            Some(
                "  HR zones: Z1 \u{2264}123: 8m \u{b7} Z2 \u{2264}137: 2m \u{b7} Z3 \u{2264}143: 2m \u{b7} Z4 \u{2264}153: 1m \u{b7} Z5 \u{2264}157: <1m"
                    .to_string()
            )
        );
    }

    #[test]
    fn zone_line_leaves_the_tops_out_when_the_lengths_differ() {
        let w = Workout {
            hr_zone_seconds: Some(vec![491, 158, 150, 110, 28, 0, 0]),
            hr_zone_max_bpm: Some(vec![123, 137, 143, 153, 157]),
            ..workout(None, None)
        };
        assert_eq!(
            format_zone_line(&w),
            Some(
                "  HR zones: Z1 8m \u{b7} Z2 2m \u{b7} Z3 2m \u{b7} Z4 1m \u{b7} Z5 <1m"
                    .to_string()
            )
        );
    }

    #[test]
    fn zone_line_is_none_without_zone_time() {
        for zones in [None, Some(vec![]), Some(vec![0, 0, 0])] {
            let w = Workout {
                hr_zone_seconds: zones.clone(),
                hr_zone_max_bpm: Some(vec![123, 137, 143]),
                ..workout(None, None)
            };
            assert_eq!(format_zone_line(&w), None, "{zones:?}");
        }
    }

    #[test]
    fn workout_line_shows_distance_and_pace() {
        assert_eq!(
            format_workout_line(&workout(Some(5043.21), Some(1.417))),
            "[2026-10-15T07:00:00] Walk (59m, 5.04 km, 11:46 /km, 287cal, avg HR 104)"
        );
    }

    #[test]
    fn workout_line_ends_with_the_training_load() {
        let w = Workout {
            training_load: Some(43),
            ..workout(Some(5043.21), Some(1.417))
        };
        assert_eq!(
            format_workout_line(&w),
            "[2026-10-15T07:00:00] Walk (59m, 5.04 km, 11:46 /km, 287cal, avg HR 104, load 43)"
        );
    }

    #[test]
    fn workout_line_leaves_out_a_missing_or_zero_pace() {
        for speed in [None, Some(0.0)] {
            assert_eq!(
                format_workout_line(&workout(Some(5043.21), speed)),
                "[2026-10-15T07:00:00] Walk (59m, 5.04 km, 287cal, avg HR 104)"
            );
        }
    }

    #[test]
    fn workout_line_without_distance_is_unchanged() {
        let w = Workout {
            name: Some("Upper Body".into()),
            activity_type: "WeightTraining".into(),
            start_time: "2026-01-02T07:30:00".into(),
            duration_seconds: 3900,
            calories: Some(320),
            avg_hr: Some(112),
            ..workout(None, None)
        };
        assert_eq!(
            format_workout_line(&w),
            "[2026-01-02T07:30:00] Upper Body \u{2014} WeightTraining (1h 05m, 320cal, avg HR 112)"
        );
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn data_commands_default_to_table() {
        for cmd in DATA_COMMANDS {
            assert_eq!(
                parse_format(&["pulse", cmd]),
                Ok(OutputFormat::Table),
                "{cmd}"
            );
        }
    }

    #[test]
    fn json_flag_selects_json() {
        for cmd in DATA_COMMANDS {
            assert_eq!(
                parse_format(&["pulse", cmd, "--json"]),
                Ok(OutputFormat::Json),
                "{cmd}"
            );
        }
    }

    #[test]
    fn format_flag_selects_format() {
        for cmd in DATA_COMMANDS {
            assert_eq!(
                parse_format(&["pulse", cmd, "--format", "json"]),
                Ok(OutputFormat::Json),
                "{cmd}"
            );
            assert_eq!(
                parse_format(&["pulse", cmd, "--format", "table"]),
                Ok(OutputFormat::Table),
                "{cmd}"
            );
            assert_eq!(
                parse_format(&["pulse", cmd, "--format", "toon"]),
                Ok(OutputFormat::Toon),
                "{cmd}"
            );
            assert_eq!(
                parse_format(&["pulse", cmd, "--format", "ison"]),
                Ok(OutputFormat::Ison),
                "{cmd}"
            );
            assert_eq!(
                parse_format(&["pulse", cmd, "--format=json"]),
                Ok(OutputFormat::Json),
                "{cmd}"
            );
        }
    }

    #[test]
    fn json_with_format_is_rejected() {
        let conflicts = [
            ["pulse", "sleep", "--json", "--format", "table"],
            ["pulse", "sleep", "--format", "table", "--json"],
            ["pulse", "sleep", "--json", "--format", "json"],
        ];
        for args in conflicts {
            assert_eq!(
                parse_format(&args),
                Err(ErrorKind::ArgumentConflict),
                "{args:?}"
            );
        }
    }

    #[test]
    fn unknown_format_is_rejected() {
        for value in ["xml", "JSON", "TOON", "ISON"] {
            assert_eq!(
                parse_format(&["pulse", "sleep", "--format", value]),
                Err(ErrorKind::InvalidValue),
                "{value}"
            );
        }
    }

    #[test]
    fn format_flags_exist_only_on_data_commands() {
        let rejected = [
            vec!["pulse", "sync", "--format", "json"],
            vec!["pulse", "sync", "--json"],
            vec!["pulse", "--format", "json", "sleep"],
            vec!["pulse", "rebuild", "--json"],
        ];
        for args in rejected {
            let kind = Cli::try_parse_from(&args).map_err(|e| e.kind()).err();
            assert_eq!(kind, Some(ErrorKind::UnknownArgument), "{args:?}");
        }
    }

    #[test]
    fn wide_flag_is_off_by_default_and_on_when_given() {
        for cmd in STORED_COMMANDS {
            assert_eq!(parse_wide(&["pulse", cmd]), Ok(false), "{cmd}");
            assert_eq!(parse_wide(&["pulse", cmd, "--wide"]), Ok(true), "{cmd}");
        }
    }

    #[test]
    fn wide_is_allowed_with_format_table() {
        for cmd in STORED_COMMANDS {
            let allowed: [&[&str]; 3] = [
                &["pulse", cmd, "--format", "table", "--wide"],
                &["pulse", cmd, "--wide", "--format=table"],
                &["pulse", cmd, "-d", "3", "--wide"],
            ];
            for args in allowed {
                assert_eq!(parse_wide(args), Ok(true), "{args:?}");
            }
        }
    }

    #[test]
    fn wide_with_another_format_is_rejected() {
        for cmd in STORED_COMMANDS {
            let conflicts: [&[&str]; 5] = [
                &["pulse", cmd, "--json", "--wide"],
                &["pulse", cmd, "--wide", "--json"],
                &["pulse", cmd, "--format", "json", "--wide"],
                &["pulse", cmd, "--wide", "--format", "toon"],
                &["pulse", cmd, "--format=ison", "--wide"],
            ];
            for args in conflicts {
                assert_eq!(
                    parse_wide(args),
                    Err(ErrorKind::ArgumentConflict),
                    "{args:?}"
                );
            }
        }
    }

    #[test]
    fn wide_conflict_error_names_the_format_and_the_command() {
        let Err(e) = parse_cli(["pulse", "heart", "--format", "toon", "--wide"], today()) else {
            panic!("--wide with --format toon must be rejected");
        };
        let message = e.render().to_string();
        assert!(
            message.contains("the argument '--wide' cannot be used with '--format toon'"),
            "{message}"
        );
        assert!(message.contains("Usage: pulse heart"), "{message}");
    }

    #[test]
    fn wide_flag_exists_only_on_stored_row_commands() {
        let rejected = [
            vec!["pulse", "vitality", "--wide"],
            vec!["pulse", "sync", "--wide"],
            vec!["pulse", "rebuild", "--wide"],
            vec!["pulse", "--wide", "sleep"],
        ];
        for args in rejected {
            let kind = parse_cli(&args, today()).map(|_| ()).map_err(|e| e.kind());
            assert_eq!(kind, Err(ErrorKind::UnknownArgument), "{args:?}");
        }
    }

    #[test]
    fn range_defaults_to_seven_dates_and_to_today_and_yesterday_for_sync() {
        for cmd in DATA_COMMANDS {
            assert_eq!(
                parse_range(&["pulse", cmd]),
                Ok(range("2026-10-11", "2026-10-17")),
                "{cmd}"
            );
        }
        for args in [&["pulse", "sync"][..], &["pulse", "sync", "garmin"]] {
            assert_eq!(
                parse_range(args),
                Ok(range("2026-10-16", "2026-10-17")),
                "{args:?}"
            );
        }
    }

    #[test]
    fn days_flag_selects_that_many_dates_ending_today() {
        for cmd in RANGE_COMMANDS {
            assert_eq!(
                parse_range(&["pulse", cmd, "-d", "3"]),
                Ok(range("2026-10-15", "2026-10-17")),
                "{cmd}"
            );
            assert_eq!(
                parse_range(&["pulse", cmd, "--days", "1"]),
                Ok(range("2026-10-17", "2026-10-17")),
                "{cmd}"
            );
            assert_eq!(
                parse_range(&["pulse", cmd, "--days", "7"]),
                Ok(range("2026-10-11", "2026-10-17")),
                "{cmd}"
            );
            assert_eq!(
                parse_range(&["pulse", cmd, "--days=-3"]),
                Err(ErrorKind::ValueValidation),
                "{cmd}"
            );
        }
    }

    #[test]
    fn days_zero_is_rejected_with_a_clear_message() {
        for cmd in RANGE_COMMANDS {
            let Err(e) = parse_cli(["pulse", cmd, "--days", "0"], today()) else {
                panic!("{cmd} --days 0 must be rejected");
            };
            assert_eq!(e.kind(), ErrorKind::ValueValidation, "{cmd}");
            let message = e.render().to_string();
            assert!(message.contains("must be at least 1"), "{cmd}: {message}");
        }
    }

    #[test]
    fn from_and_to_select_that_range() {
        for cmd in RANGE_COMMANDS {
            assert_eq!(
                parse_range(&["pulse", cmd, "--from", "2026-10-01", "--to", "2026-10-07"]),
                Ok(range("2026-10-01", "2026-10-07")),
                "{cmd}"
            );
            assert_eq!(
                parse_range(&["pulse", cmd, "--from", "2026-10-01"]),
                Ok(range("2026-10-01", "2026-10-17")),
                "{cmd}"
            );
            assert_eq!(
                parse_range(&["pulse", cmd, "--from=2026-10-17", "--to=2026-10-17"]),
                Ok(range("2026-10-17", "2026-10-17")),
                "{cmd}"
            );
        }
    }

    #[test]
    fn days_cannot_be_combined_with_from_or_to() {
        for cmd in RANGE_COMMANDS {
            let conflicts: [&[&str]; 4] = [
                &["pulse", cmd, "-d", "3", "--from", "2026-10-01"],
                &["pulse", cmd, "--from", "2026-10-01", "--days", "3"],
                &["pulse", cmd, "--days", "3", "--to", "2026-10-07"],
                &[
                    "pulse",
                    cmd,
                    "--days",
                    "3",
                    "--from",
                    "2026-10-01",
                    "--to",
                    "2026-10-07",
                ],
            ];
            for args in conflicts {
                assert_eq!(
                    parse_range(args),
                    Err(ErrorKind::ArgumentConflict),
                    "{args:?}"
                );
            }
        }
    }

    #[test]
    fn to_needs_from() {
        for cmd in RANGE_COMMANDS {
            assert_eq!(
                parse_range(&["pulse", cmd, "--to", "2026-10-07"]),
                Err(ErrorKind::MissingRequiredArgument),
                "{cmd}"
            );
        }
    }

    #[test]
    fn malformed_dates_are_rejected() {
        for cmd in ["sleep", "sync"] {
            let malformed: [&[&str]; 4] = [
                &["pulse", cmd, "--from", "2026-13-01"],
                &["pulse", cmd, "--from", "2026-1-5"],
                &["pulse", cmd, "--from", "+10000-01-01"],
                &["pulse", cmd, "--from", "2026-10-01", "--to", "yesterday"],
            ];
            for args in malformed {
                assert_eq!(
                    parse_range(args),
                    Err(ErrorKind::ValueValidation),
                    "{args:?}"
                );
            }
        }
    }

    #[test]
    fn from_after_to_error_names_both_flags_and_the_command() {
        let args = [
            "pulse",
            "vitality",
            "--from",
            "2026-10-08",
            "--to",
            "2026-10-01",
        ];
        let Err(e) = parse_cli(args, today()) else {
            panic!("--from after --to must be rejected");
        };
        assert_eq!(e.kind(), ErrorKind::ValueValidation);
        let message = e.render().to_string();
        assert!(
            message.contains("--from 2026-10-08 is after --to 2026-10-01"),
            "{message}"
        );
        assert!(message.contains("Usage: pulse vitality"), "{message}");
    }

    #[test]
    fn from_after_today_is_rejected() {
        let Err(e) = parse_cli(["pulse", "sleep", "--from", "2026-10-18"], today()) else {
            panic!("--from after today must be rejected");
        };
        assert_eq!(e.kind(), ErrorKind::ValueValidation);
        let message = e.render().to_string();
        assert!(
            message.contains("--from 2026-10-18 is after today (2026-10-17)"),
            "{message}"
        );
    }

    #[test]
    fn read_commands_reject_a_range_that_ends_after_today() {
        for command in ["sleep", "vitality"] {
            let args = [
                "pulse",
                command,
                "--from",
                "2026-10-01",
                "--to",
                "2026-10-31",
            ];
            let Err(e) = parse_cli(args, today()) else {
                panic!("{command} --to after today must be rejected");
            };
            assert_eq!(e.kind(), ErrorKind::ValueValidation);
            let message = e.render().to_string();
            assert!(
                message.contains("--to 2026-10-31 is after today (2026-10-17)"),
                "{message}"
            );
            assert!(!message.contains("sync cannot"), "{message}");
        }
    }

    #[test]
    fn read_commands_accept_a_range_ending_today() {
        assert_eq!(
            parse_range(&[
                "pulse",
                "sleep",
                "--from",
                "2026-10-01",
                "--to",
                "2026-10-17"
            ]),
            Ok(range("2026-10-01", "2026-10-17"))
        );
    }

    #[test]
    fn sync_rejects_a_range_that_ends_after_today() {
        let args = [
            "pulse",
            "sync",
            "garmin",
            "--from",
            "2026-10-01",
            "--to",
            "2026-10-18",
        ];
        let Err(e) = parse_cli(args, today()) else {
            panic!("sync --to after today must be rejected");
        };
        assert_eq!(e.kind(), ErrorKind::ValueValidation);
        let message = e.render().to_string();
        assert!(
            message.contains("--to 2026-10-18 is after today (2026-10-17)"),
            "{message}"
        );
        assert!(message.contains("Usage: pulse sync"), "{message}");

        let args = [
            "pulse",
            "sync",
            "garmin",
            "--from",
            "2026-10-01",
            "--to",
            "2026-10-17",
        ];
        let Ok(cli) = parse_cli(args, today()) else {
            panic!("sync --to today must be accepted");
        };
        let Commands::Sync {
            provider,
            range: flags,
        } = cli.command
        else {
            panic!("not a sync command");
        };
        assert_eq!(provider.as_deref(), Some("garmin"));
        assert_eq!(
            flags.resolve(today()).unwrap(),
            range("2026-10-01", "2026-10-17")
        );
    }

    #[test]
    fn range_flags_work_with_every_output_flag() {
        for cmd in STORED_COMMANDS {
            let toon = ["pulse", cmd, "--from", "2026-10-01", "--to", "2026-10-07"];
            let toon: Vec<&str> = toon.into_iter().chain(["--format", "toon"]).collect();
            assert_eq!(parse_format(&toon), Ok(OutputFormat::Toon), "{cmd}");
            assert_eq!(
                parse_range(&toon),
                Ok(range("2026-10-01", "2026-10-07")),
                "{cmd}"
            );
            assert_eq!(
                parse_wide(&["pulse", cmd, "--wide", "--from", "2026-10-01"]),
                Ok(true),
                "{cmd}"
            );
            assert_eq!(
                parse_format(&["pulse", cmd, "--json", "-d", "3"]),
                Ok(OutputFormat::Json),
                "{cmd}"
            );
        }
        assert_eq!(
            parse_format(&[
                "pulse",
                "vitality",
                "--from",
                "2026-10-01",
                "--format",
                "ison"
            ]),
            Ok(OutputFormat::Ison)
        );
    }

    #[test]
    fn range_flags_exist_only_on_data_commands_and_sync() {
        let rejected = [
            vec!["pulse", "rebuild", "--from", "2026-10-01"],
            vec!["pulse", "rebuild", "-d", "3"],
            vec!["pulse", "config", "--from", "2026-10-01"],
            vec!["pulse", "garmin-login", "--to", "2026-10-01"],
            vec!["pulse", "--from", "2026-10-01", "sleep"],
        ];
        for args in rejected {
            let kind = parse_cli(&args, today()).map(|_| ()).map_err(|e| e.kind());
            assert_eq!(kind, Err(ErrorKind::UnknownArgument), "{args:?}");
        }
    }

    #[test]
    fn rebuild_takes_no_arguments() {
        let cli = Cli::try_parse_from(["pulse", "rebuild"]).unwrap();
        assert!(matches!(cli.command, Commands::Rebuild));

        let kind = Cli::try_parse_from(["pulse", "rebuild", "garmin"])
            .map(|_| ())
            .map_err(|e| e.kind());
        assert_eq!(kind, Err(ErrorKind::UnknownArgument));
    }
}
