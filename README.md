# Pulse

Pulse is a unified health data aggregator that pulls data from Garmin Connect and Intervals.icu into a local SQLite database, then surfaces actionable insights through a Rust CLI. It computes a daily **vitality score** — a weighted composite of sleep, recovery, activity, stress, and fitness — so you can track your long-term health trajectory without trusting a vendor dashboard.

## Features

- **Sync** data from Garmin Connect (sleep, heart rate, HRV, VO2 max, Body Battery, steps, stress, strength exercise sets) and Intervals.icu (workouts, weight, body fat, fitness and fatigue)
- **Vitality score** — weighted daily composite with configurable targets
- **Fitness scoring** — VO2 max percentiles (Shvartz & Reinbold norms) + FFMI from lean body mass
- **Raw data tables** — sleep, heart rate, recovery, activity, stress, workouts, wellness; `--wide` shows every stored column
- **Machine-readable output** — `--format <table|json|toon|ison>` on all data commands; `--json` is shorthand for `--format json`, and TOON and ISON are compact formats that suit LLM input
- **Date ranges** — `--days N`, or `--from` and `--to` (`YYYY-MM-DD`) for a fixed range, on every data command and `sync`
- **TUI** — planned interactive terminal dashboard

## Quick Start

```bash
# 1. Install (puts `pulse` in ~/.cargo/bin)
cargo install pulse-cli

# 2. Initialise config (creates ~/.pulse/config.toml)
pulse config init

# 3. Authenticate with Garmin (optional; prompts for the password)
pulse garmin-login -u you@example.com

# 4. Sync and view vitality
pulse sync
pulse vitality
```

## Requirements

| Requirement | Version |
|---|---|
| Rust (stable) | 1.88+ |
| Garmin Connect account | optional |
| Intervals.icu API key | optional |

## Documentation

- [Installation](wiki/installation.md)
- [Configuration](wiki/configuration.md)
- [Usage](wiki/usage.md)
- [Syncing data](wiki/syncing.md)
- [Vitality score](wiki/vitality-score.md)

## License

MIT
