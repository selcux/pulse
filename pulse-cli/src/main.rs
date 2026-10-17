mod output;
mod table;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use output::{OutputArgs, OutputFormat, Rows, StoredOutputArgs, VitalityReport, WorkoutLog};
use pulse_health_core::config;
use pulse_health_core::db::Database;
use pulse_health_core::db::queries;
use pulse_health_core::models::{ExerciseSet, Workout};
use pulse_health_core::sync;
use pulse_health_core::vitality;
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
        /// Number of days to sync
        #[arg(short, long, default_value_t = 1)]
        days: u32,
    },
    /// Refill the data tables from stored provider responses (no network)
    Rebuild,
    /// Show sleep duration, stages, scores, and bed and wake times
    Sleep {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show resting heart rate, min/max HR, HRV, and VO2 max
    Heart {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show body battery peak/low values
    Recovery {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show steps, active minutes, floors, calories, and distance
    Activity {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show stress levels and time at each stress level
    Stress {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show workouts and exercise sets
    Workouts {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: StoredOutputArgs,
    },
    /// Show composite vitality score (sleep + recovery + activity + stress)
    Vitality {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
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
    /// Subcommand name and output flags of the six commands that print stored rows; `None` for the others.
    fn stored_output(&self) -> Option<(&'static str, StoredOutputArgs)> {
        match self {
            Commands::Sleep { output, .. } => Some(("sleep", *output)),
            Commands::Heart { output, .. } => Some(("heart", *output)),
            Commands::Recovery { output, .. } => Some(("recovery", *output)),
            Commands::Activity { output, .. } => Some(("activity", *output)),
            Commands::Stress { output, .. } => Some(("stress", *output)),
            Commands::Workouts { output, .. } => Some(("workouts", *output)),
            _ => None,
        }
    }
}

/// Parses the command line, then rejects `--wide` with a `--format` other than `table`, which clap
/// cannot express (`--json --wide` is rejected by clap itself, through `conflicts_with`).
fn parse_cli<I, T>(args: I) -> Result<Cli, clap::Error>
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
        let mut cmd = Cli::command();
        cmd.build();
        let sub = cmd
            .find_subcommand_mut(name)
            .expect("stored_output names a subcommand");
        return Err(sub.error(
            ErrorKind::ArgumentConflict,
            format!("the argument '--wide' cannot be used with '--format {format_name}'"),
        ));
    }
    Ok(cli)
}

fn main() -> anyhow::Result<()> {
    let cli = parse_cli(std::env::args_os()).unwrap_or_else(|e| e.exit());

    match cli.command {
        Commands::GarminLogin { username, mfa } => cmd_garmin_login(username, mfa),
        Commands::Sync { provider, days } => cmd_sync(provider, days),
        Commands::Rebuild => cmd_rebuild(),
        Commands::Sleep { days, output } => cmd_sleep(days, output.format(), output.wide()),
        Commands::Heart { days, output } => cmd_heart(days, output.format(), output.wide()),
        Commands::Recovery { days, output } => cmd_recovery(days, output.format(), output.wide()),
        Commands::Activity { days, output } => cmd_activity(days, output.format(), output.wide()),
        Commands::Stress { days, output } => cmd_stress(days, output.format(), output.wide()),
        Commands::Workouts { days, output } => cmd_workouts(days, output.format(), output.wide()),
        Commands::Vitality { days, output } => cmd_vitality(days, output.format()),
        Commands::Config { action } => cmd_config(action),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_sync(provider: Option<String>, days: u32) -> anyhow::Result<()> {
    let cfg = config::load_config()?;
    let db = Database::open()?;

    // The first sync that stores a response says so once; no state is kept for it.
    let store_was_empty = cfg.storage.raw_responses && queries::count_raw_responses(&db)? == 0;

    match provider {
        Some(name) => {
            println!("Syncing {}...", name);
            match sync::sync_provider(&db, &cfg, &name, days) {
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
            let report = sync::sync_all(&db, &cfg, days)?;
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

fn cmd_sleep(days: i32, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_sleep(&db, days)?;

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
        println!("No sleep data found for the last {} days.", days);
        return Ok(());
    }

    println!("{}", table::SLEEP.render(&data, wide));

    Ok(())
}

fn cmd_heart(days: i32, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_heart(&db, days)?;

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
        println!("No heart data found for the last {} days.", days);
        return Ok(());
    }

    println!("{}", table::HEART.render(&data, wide));

    Ok(())
}

fn cmd_recovery(days: i32, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_recovery(&db, days)?;

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
        println!("No recovery data found for the last {} days.", days);
        return Ok(());
    }

    println!("{}", table::RECOVERY.render(&data, wide));

    Ok(())
}

fn cmd_activity(days: i32, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_activity(&db, days)?;

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
        println!("No activity data found for the last {} days.", days);
        return Ok(());
    }

    println!("{}", table::ACTIVITY.render(&data, wide));

    Ok(())
}

fn cmd_stress(days: i32, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_stress(&db, days)?;

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
        println!("No stress data found for the last {} days.", days);
        return Ok(());
    }

    println!("{}", table::STRESS.render(&data, wide));

    Ok(())
}

fn cmd_workouts(days: i32, format: OutputFormat, wide: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let workouts = queries::query_workouts(&db, days)?;
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
        println!("No workouts found for the last {} days.", days);
        return Ok(());
    }

    if wide {
        println!("{}", table::workout_log_wide(&entries));
        return Ok(());
    }

    for (w, sets) in &entries {
        let duration = format_duration(w.duration_seconds);
        let cal = match w.calories {
            Some(c) => format!("{}cal", c),
            None => "-".into(),
        };
        let hr = match w.avg_hr {
            Some(h) => format!("avg HR {}", h),
            None => "no HR".into(),
        };
        let label = match &w.name {
            Some(n) => format!("{} \u{2014} {}", n, w.activity_type),
            None => w.activity_type.clone(),
        };
        println!(
            "[{}] {} ({}, {}, {})",
            w.start_time, label, duration, cal, hr
        );

        for s in sets {
            println!("{}", format_exercise_set(s));
        }
        if !sets.is_empty() {
            println!();
        }
    }

    Ok(())
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

fn cmd_vitality(days: i32, format: OutputFormat) -> anyhow::Result<()> {
    let cfg = config::load_config()?;
    let db = Database::open()?;
    let scores = vitality::calculate_vitality(&db, &cfg, days)?;
    let pace = vitality::calculate_pace(&scores);

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

    if scores.is_empty() {
        println!(
            "No vitality data for the last {} days. (Sleep data is required as anchor.)",
            days
        );
        return Ok(());
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

    println!("{}", table::render(table::VITALITY_COLUMNS, &scores));

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

    const DATA_COMMANDS: [&str; 7] = [
        "sleep", "heart", "recovery", "activity", "stress", "workouts", "vitality",
    ];

    const STORED_COMMANDS: [&str; 6] = [
        "sleep", "heart", "recovery", "activity", "stress", "workouts",
    ];

    fn parse_format(args: &[&str]) -> Result<OutputFormat, ErrorKind> {
        let cli = parse_cli(args).map_err(|e| e.kind())?;
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
        let cli = parse_cli(args).map_err(|e| e.kind())?;
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
        let Err(e) = parse_cli(["pulse", "heart", "--format", "toon", "--wide"]) else {
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
            let kind = parse_cli(&args).map(|_| ()).map_err(|e| e.kind());
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
