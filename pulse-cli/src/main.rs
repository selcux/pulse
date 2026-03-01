use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pulse", about = "Health data unification CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync health data from providers
    Sync {
        /// Number of days to sync (default: 7)
        #[arg(short, long, default_value_t = 7)]
        days: i32,
    },
    /// Show today's health summary
    Summary,
    /// Show configuration
    Config,
    /// Show vitality score
    Vitality,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sync { days } => {
            println!("TODO: Sync last {} days", days);
        }
        Commands::Summary => {
            println!("TODO: Show today's summary");
        }
        Commands::Config => {
            let config = pulse_core::config::load_config()?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        Commands::Vitality => {
            println!("TODO: Show vitality score");
        }
    }

    Ok(())
}
