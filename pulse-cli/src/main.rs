use clap::{Parser, Subcommand};

use pulse_core::config;
use pulse_core::db::queries;
use pulse_core::db::Database;
use pulse_core::models::ExerciseSet;
use pulse_core::sync;
use pulse_core::vitality;

#[derive(Parser)]
#[command(name = "pulse", about = "Health data unification system", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync health data from providers
    Sync {
        /// Provider to sync (omit for all)
        provider: Option<String>,
        /// Number of days to sync
        #[arg(short, long, default_value_t = 1)]
        days: u32,
    },
    /// Show sleep data
    Sleep {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[arg(long)]
        json: bool,
    },
    /// Show heart/HRV data
    Heart {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[arg(long)]
        json: bool,
    },
    /// Show body battery/recovery data
    Recovery {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[arg(long)]
        json: bool,
    },
    /// Show steps/activity data
    Activity {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[arg(long)]
        json: bool,
    },
    /// Show stress data
    Stress {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[arg(long)]
        json: bool,
    },
    /// Show workouts and exercise sets
    Workouts {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[arg(long)]
        json: bool,
    },
    /// Show composite vitality score
    Vitality {
        #[arg(short, long, default_value_t = 7)]
        days: i32,
        #[arg(long)]
        json: bool,
    },
    /// Configuration management
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
        Commands::Sync { provider, days } => cmd_sync(provider, days),
        Commands::Sleep { days, json } => cmd_sleep(days, json),
        Commands::Heart { days, json } => cmd_heart(days, json),
        Commands::Recovery { days, json } => cmd_recovery(days, json),
        Commands::Activity { days, json } => cmd_activity(days, json),
        Commands::Stress { days, json } => cmd_stress(days, json),
        Commands::Workouts { days, json } => cmd_workouts(days, json),
        Commands::Vitality { days, json } => cmd_vitality(days, json),
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
                    println!(
                        "  {} records synced",
                        report.records_synced
                    );
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
                    println!("Syncing {}... {} records synced", r.provider, r.records_synced);
                } else {
                    println!(
                        "Syncing {}... error: {}",
                        r.provider,
                        r.errors.join("; ")
                    );
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

fn cmd_sleep(days: i32, json: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_sleep(&db, days)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    if data.is_empty() {
        println!("No sleep data found for the last {} days.", days);
        return Ok(());
    }

    println!(
        "{:<12}{sep}{:>8}{sep}{:>8}{sep}{:>8}{sep}{:>8}{sep}{:>7}{sep}{:>7}{sep}{:>6}",
        "Date", "Total", "Deep", "REM", "Light", "Awake", "Score", "HRV",
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

fn cmd_heart(days: i32, json: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_heart(&db, days)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    if data.is_empty() {
        println!("No heart data found for the last {} days.", days);
        return Ok(());
    }

    println!(
        "{:<12}{sep}{:>10}{sep}{:>6}{sep}{:>6}{sep}{:>8}",
        "Date", "Resting HR", "Max", "Min", "HRV avg",
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

fn cmd_recovery(days: i32, json: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_recovery(&db, days)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    if data.is_empty() {
        println!("No recovery data found for the last {} days.", days);
        return Ok(());
    }

    println!(
        "{:<12}{sep}{:>7}{sep}{:>7}{sep}{:>7}{sep}{:>7}",
        "Date", "Peak", "Low", "Chrgd", "Draind",
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

fn cmd_activity(days: i32, json: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_activity(&db, days)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    if data.is_empty() {
        println!("No activity data found for the last {} days.", days);
        return Ok(());
    }

    println!(
        "{:<12}{sep}{:>7}{sep}{:>10}{sep}{:>6}",
        "Date", "Steps", "Active min", "Floors",
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

fn cmd_stress(days: i32, json: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let data = queries::query_stress(&db, days)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    if data.is_empty() {
        println!("No stress data found for the last {} days.", days);
        return Ok(());
    }

    println!(
        "{:<12}{sep}{:>10}{sep}{:>10}",
        "Date", "Avg Stress", "Max Stress",
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

fn cmd_workouts(days: i32, json: bool) -> anyhow::Result<()> {
    let db = Database::open()?;
    let workouts = queries::query_workouts(&db, days)?;

    if json {
        // Build a combined structure with sets included
        let mut combined: Vec<serde_json::Value> = Vec::new();
        for w in &workouts {
            let sets = queries::query_exercise_sets(&db, &w.id)?;
            let mut val = serde_json::to_value(w)?;
            val["exercise_sets"] = serde_json::to_value(&sets)?;
            combined.push(val);
        }
        println!("{}", serde_json::to_string_pretty(&combined)?);
        return Ok(());
    }

    if workouts.is_empty() {
        println!("No workouts found for the last {} days.", days);
        return Ok(());
    }

    for w in &workouts {
        let duration = format_duration(w.duration_seconds);
        let cal = match w.calories {
            Some(c) => format!("{}cal", c),
            None => "-".into(),
        };
        let hr = match w.avg_hr {
            Some(h) => format!("avg HR {}", h),
            None => "no HR".into(),
        };
        println!(
            "[{}] {} ({}, {}, {})",
            w.start_time, w.activity_type, duration, cal, hr
        );

        let sets = queries::query_exercise_sets(&db, &w.id)?;
        for s in &sets {
            print_exercise_set(s);
        }
        if !sets.is_empty() {
            println!();
        }
    }

    Ok(())
}

fn print_exercise_set(s: &ExerciseSet) {
    let reps = match s.repetitions {
        Some(r) => format!("{}", r),
        None => "-".into(),
    };
    let weight = match s.weight_kg {
        Some(w) => format!("{:.1}kg", w),
        None => "BW".into(),
    };
    println!(
        "  {:>2}. {:<20} {:>3} x {}",
        s.set_order, s.exercise_name, reps, weight
    );
}

fn cmd_vitality(days: i32, json: bool) -> anyhow::Result<()> {
    let cfg = config::load_config()?;
    let db = Database::open()?;
    let scores = vitality::calculate_vitality(&db, &cfg, days)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&scores)?);
        return Ok(());
    }

    if scores.is_empty() {
        println!("No vitality data for the last {} days. (Sleep data is required as anchor.)", days);
        return Ok(());
    }

    println!(
        "{:<12}{sep}{:>7}{sep}{:>7}{sep}{:>10}{sep}{:>10}{sep}{:>8}",
        "Date", "Total", "Sleep", "Recovery", "Activity", "Stress",
        sep = " \u{2502} "
    );
    println!(
        "{:\u{2500}<12}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>9}\u{253c}{:\u{2500}>12}\u{253c}{:\u{2500}>12}\u{253c}{:\u{2500}>10}",
        "", "", "", "", "", ""
    );
    for s in &scores {
        println!(
            "{:<12}{sep}{:>7.1}{sep}{:>7.1}{sep}{:>10.1}{sep}{:>10.1}{sep}{:>8.1}",
            s.date,
            s.total_score,
            s.sleep_score,
            s.recovery_score,
            s.activity_score,
            s.stress_score,
            sep = " \u{2502} "
        );
    }

    Ok(())
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
