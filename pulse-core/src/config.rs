use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulseConfig {
    pub user: UserConfig,
    pub providers: ProvidersConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub age: Option<i32>,
    pub sleep_target_hours: Option<f64>,
    pub steps_target: Option<i32>,
    // TODO: Remove rhr_baseline and hrv_baseline once Garmin sync is live —
    // auto-compute from 30-day DB averages only, no manual config needed.
    pub rhr_baseline: Option<f64>,
    pub hrv_baseline: Option<f64>,
    pub vo2_max: Option<f64>,
    pub lean_body_mass_kg: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvidersConfig {
    pub garmin: Option<GarminConfig>,
    pub intervals: Option<IntervalsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarminConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalsConfig {
    pub enabled: bool,
    pub api_key: String,
    pub athlete_id: String,
}

/// Returns the pulse config directory: ~/.pulse/
pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".pulse")
}

/// Returns a default configuration with sensible defaults.
pub fn default_config() -> PulseConfig {
    PulseConfig {
        user: UserConfig {
            age: None,
            sleep_target_hours: Some(8.0),
            steps_target: Some(10_000),
            rhr_baseline: None,
            hrv_baseline: None,
            vo2_max: None,
            lean_body_mass_kg: None,
        },
        providers: ProvidersConfig {
            garmin: Some(GarminConfig { enabled: false }),
            intervals: None,
        },
    }
}

/// Load config from ~/.pulse/config.toml, creating a default if it doesn't exist.
pub fn load_config() -> Result<PulseConfig> {
    let config_path = config_dir().join("config.toml");

    if !config_path.exists() {
        write_default_config().context("Failed to write default config")?;
    }

    let contents =
        std::fs::read_to_string(&config_path).context("Failed to read config.toml")?;

    let config: PulseConfig =
        toml::from_str(&contents).context("Failed to parse config.toml")?;

    Ok(config)
}

/// Write default config.toml to ~/.pulse/config.toml and return the path.
pub fn write_default_config() -> Result<PathBuf> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).context("Failed to create config directory")?;

    let config_path = dir.join("config.toml");
    let config = default_config();
    let contents = toml::to_string_pretty(&config).context("Failed to serialize config")?;

    std::fs::write(&config_path, contents).context("Failed to write config.toml")?;

    Ok(config_path)
}
