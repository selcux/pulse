//! Configuration loading and defaults for `~/.pulse/config.toml`.
//!
//! On first run, [`load_config`] creates a default config file. Edit it
//! to enable providers, set personal baselines, and tune scoring targets.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level configuration, serialised as `~/.pulse/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulseConfig {
    pub user: UserConfig,
    pub providers: ProvidersConfig,
}

/// Personal targets and optional manual baselines used for vitality scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    /// Your age in years — currently reserved for future norm lookups.
    pub age: Option<i32>,
    /// Target sleep duration in hours. Defaults to `8.0`.
    pub sleep_target_hours: Option<f64>,
    /// Daily step target. Defaults to `10_000`.
    pub steps_target: Option<i32>,
    /// Manual resting-heart-rate baseline (bpm). If unset, the 30-day DB
    /// average is used automatically once enough data is synced.
    pub rhr_baseline: Option<f64>,
    /// Manual HRV baseline (ms). Same auto-compute fallback as `rhr_baseline`.
    pub hrv_baseline: Option<f64>,
    /// Manual VO2 Max fallback (ml/kg/min). Used when the device does not
    /// report VO2 Max on a given day.
    pub vo2_max: Option<f64>,
    /// Lean body mass in kg. Enables FFMI-based fitness scoring when combined
    /// with `height_cm`.
    pub lean_body_mass_kg: Option<f64>,
    /// Height in centimetres. Used with `lean_body_mass_kg` for FFMI scoring.
    pub height_cm: Option<f64>,
}

/// Enabled data-source providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvidersConfig {
    pub garmin: Option<GarminConfig>,
    pub intervals: Option<IntervalsConfig>,
}

/// Garmin Connect provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarminConfig {
    /// Set to `true` to include Garmin in sync operations.
    pub enabled: bool,
    /// Garmin Connect username/email. If omitted, `pulse garmin-login` will prompt.
    pub username: Option<String>,
}

/// Intervals.icu provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalsConfig {
    /// Set to `true` to include Intervals.icu in sync operations.
    pub enabled: bool,
    /// Intervals.icu API key (found in Settings → API).
    pub api_key: String,
    /// Your Intervals.icu athlete ID (the `iXXXXXX` string in your profile URL).
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
            height_cm: None,
        },
        providers: ProvidersConfig {
            garmin: Some(GarminConfig {
                enabled: false,
                username: None,
            }),
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
