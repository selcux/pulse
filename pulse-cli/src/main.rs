mod output;

use clap::{Parser, Subcommand};

use output::{OutputArgs, OutputFormat, Rows, VitalityReport, WorkoutLog};
use pulse_health_core::config;
use pulse_health_core::db::Database;
use pulse_health_core::db::queries;
use pulse_health_core::models::{ExerciseSet, Workout};
use pulse_health_core::sync;
use pulse_health_core::vitality;

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
    /// Show sleep duration, stages, and scores
    Sleep {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Show resting heart rate, min/max HR, and HRV
    Heart {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Show body battery peak/low values
    Recovery {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Show steps, active minutes, and floors
    Activity {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Show average and max stress levels
    Stress {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Show workouts and exercise sets
    Workouts {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[command(flatten)]
        output: OutputArgs,
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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::GarminLogin { username, mfa } => cmd_garmin_login(username, mfa),
        Commands::Sync { provider, days } => cmd_sync(provider, days),
        Commands::Sleep { days, output } => cmd_sleep(days, output.format()),
        Commands::Heart { days, output } => cmd_heart(days, output.format()),
        Commands::Recovery { days, output } => cmd_recovery(days, output.format()),
        Commands::Activity { days, output } => cmd_activity(days, output.format()),
        Commands::Stress { days, output } => cmd_stress(days, output.format()),
        Commands::Workouts { days, output } => cmd_workouts(days, output.format()),
        Commands::Vitality { days, output } => cmd_vitality(days, output.format()),
        Commands::Config { action } => cmd_config(action),
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_duration(seconds: i64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{}h {:02}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

fn opt_i32(val: Option<i32>) -> String {
    match val {
        Some(v) => v.to_string(),
        None => "-".into(),
    }
}

fn opt_f64(val: Option<f64>, decimals: usize) -> String {
    match val {
        Some(v) => format!("{v:.decimals$}"),
        None => "-".into(),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_sync(provider: Option<String>, days: u32) -> anyhow::Result<()> {
    let cfg = config::load_config()?;
    let db = Database::open()?;

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

    Ok(())
}

fn cmd_sleep(days: i32, format: OutputFormat) -> anyhow::Result<()> {
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

    println!(
        "{:<12}{sep}{:>8}{sep}{:>8}{sep}{:>8}{sep}{:>8}{sep}{:>7}{sep}{:>7}{sep}{:>6}",
        "Date",
        "Total",
        "Deep",
        "REM",
        "Light",
        "Awake",
        "Score",
        "HRV",
        sep = " \u{2502} "
    );
    println!(
        "{:\u{2500}<12}\u{253c}{:\u{2500}>10}\u{253c}{:\u{2500}>10}\u{253c}{:\u{2500}>10}\u{253c}{:\u{2500}>10}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>8}",
        "", "", "", "", "", "", "", ""
    );
    for s in &data {
        println!(
            "{:<12}{sep}{:>8}{sep}{:>8}{sep}{:>8}{sep}{:>8}{sep}{:>7}{sep}{:>7}{sep}{:>6}",
            s.date,
            format_duration(s.total_seconds),
            format_duration(s.deep_seconds),
            format_duration(s.rem_seconds),
            format_duration(s.light_seconds),
            format_duration(s.awake_seconds),
            opt_i32(s.sleep_score),
            opt_f64(s.hrv_ms, 1),
            sep = " \u{2502} "
        );
    }

    Ok(())
}

fn cmd_heart(days: i32, format: OutputFormat) -> anyhow::Result<()> {
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

    println!(
        "{:<12}{sep}{:>10}{sep}{:>6}{sep}{:>6}{sep}{:>8}",
        "Date",
        "Resting HR",
        "Max",
        "Min",
        "HRV avg",
        sep = " \u{2502} "
    );
    println!(
        "{:\u{2500}<12}\u{253c}{:\u{2500}>12}\u{253c}{:\u{2500}>8}\u{253c}{:\u{2500}>8}\u{253c}{:\u{2500}>10}",
        "", "", "", "", ""
    );
    for h in &data {
        println!(
            "{:<12}{sep}{:>10}{sep}{:>6}{sep}{:>6}{sep}{:>8}",
            h.date,
            opt_i32(h.resting_hr),
            opt_i32(h.max_hr),
            opt_i32(h.min_hr),
            opt_f64(h.hrv_avg, 1),
            sep = " \u{2502} "
        );
    }

    Ok(())
}

fn cmd_recovery(days: i32, format: OutputFormat) -> anyhow::Result<()> {
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

    println!(
        "{:<12}{sep}{:>7}{sep}{:>7}{sep}{:>7}{sep}{:>7}",
        "Date",
        "Peak",
        "Low",
        "Chrgd",
        "Draind",
        sep = " \u{2502} "
    );
    println!(
        "{:\u{2500}<12}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>9}",
        "", "", "", "", ""
    );
    for r in &data {
        println!(
            "{:<12}{sep}{:>7}{sep}{:>7}{sep}{:>7}{sep}{:>7}",
            r.date,
            opt_i32(r.body_battery_peak),
            opt_i32(r.body_battery_low),
            opt_i32(r.body_battery_charged),
            opt_i32(r.body_battery_drained),
            sep = " \u{2502} "
        );
    }

    Ok(())
}

fn cmd_activity(days: i32, format: OutputFormat) -> anyhow::Result<()> {
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

    println!(
        "{:<12}{sep}{:>7}{sep}{:>10}{sep}{:>6}",
        "Date",
        "Steps",
        "Active min",
        "Floors",
        sep = " \u{2502} "
    );
    println!(
        "{:\u{2500}<12}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>12}\u{253c}{:\u{2500}>8}",
        "", "", "", ""
    );
    for a in &data {
        println!(
            "{:<12}{sep}{:>7}{sep}{:>10}{sep}{:>6}",
            a.date,
            opt_i32(a.steps),
            opt_i32(a.active_minutes),
            opt_i32(a.floors),
            sep = " \u{2502} "
        );
    }

    Ok(())
}

fn cmd_stress(days: i32, format: OutputFormat) -> anyhow::Result<()> {
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

    println!(
        "{:<12}{sep}{:>10}{sep}{:>10}",
        "Date",
        "Avg Stress",
        "Max Stress",
        sep = " \u{2502} "
    );
    println!(
        "{:\u{2500}<12}\u{253c}{:\u{2500}>12}\u{253c}{:\u{2500}>12}",
        "", "", ""
    );
    for s in &data {
        println!(
            "{:<12}{sep}{:>10}{sep}{:>10}",
            s.date,
            opt_i32(s.avg_stress),
            opt_i32(s.max_stress),
            sep = " \u{2502} "
        );
    }

    Ok(())
}

fn cmd_workouts(days: i32, format: OutputFormat) -> anyhow::Result<()> {
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
    let weight = match s.weight_kg {
        Some(w) => format!("{:.1}kg", w),
        None => "BW".into(),
    };
    format!(
        "  {:>2}. {:<20} {:>3} x {}",
        s.set_order,
        exercise_label(s),
        reps,
        weight
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

    println!(
        "{:<12}{sep}{:>7}{sep}{:>7}{sep}{:>10}{sep}{:>10}{sep}{:>8}{sep}{:>9}",
        "Date",
        "Total",
        "Sleep",
        "Recovery",
        "Activity",
        "Stress",
        "Fitness",
        sep = " \u{2502} "
    );
    println!(
        "{:\u{2500}<12}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>12}\u{253c}{:\u{2500}>12}\u{253c}{:\u{2500}>10}\u{253c}{:\u{2500}>11}",
        "", "", "", "", "", "", ""
    );
    for s in &scores {
        let fitness = match s.fitness_score {
            Some(f) => format!("{:>9.1}", f),
            None => format!("{:>9}", "-"),
        };
        println!(
            "{:<12}{sep}{:>7.1}{sep}{:>7.1}{sep}{:>10.1}{sep}{:>10.1}{sep}{:>8.1}{sep}{}",
            s.date,
            s.total_score,
            s.sleep_score,
            s.recovery_score,
            s.activity_score,
            s.stress_score,
            fitness,
            sep = " \u{2502} "
        );
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
    use clap::CommandFactory;
    use clap::error::ErrorKind;
    use output::OutputFormat;

    const DATA_COMMANDS: [&str; 7] = [
        "sleep", "heart", "recovery", "activity", "stress", "workouts", "vitality",
    ];

    fn parse_format(args: &[&str]) -> Result<OutputFormat, ErrorKind> {
        let cli = Cli::try_parse_from(args).map_err(|e| e.kind())?;
        match cli.command {
            Commands::Sleep { output, .. }
            | Commands::Heart { output, .. }
            | Commands::Recovery { output, .. }
            | Commands::Activity { output, .. }
            | Commands::Stress { output, .. }
            | Commands::Workouts { output, .. }
            | Commands::Vitality { output, .. } => Ok(output.format()),
            _ => panic!("not a data command"),
        }
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
        ];
        for args in rejected {
            let kind = Cli::try_parse_from(&args).map_err(|e| e.kind()).err();
            assert_eq!(kind, Some(ErrorKind::UnknownArgument), "{args:?}");
        }
    }
}
