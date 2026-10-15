# Configuration

Config lives at `~/.pulse/config.toml`. Run `pulse config init` to generate the file with defaults, then edit as needed.

## `[user]`

Personal targets and biometrics used for scoring.

| Field | Type | Default | Description |
|---|---|---|---|
| `sleep_target_hours` | float | `8.0` | Sleep duration target for scoring (hours) |
| `steps_target` | int | `10000` | Daily step goal |
| `vo2_max` | float | — | Fallback VO2 max (ml/kg/min), used when no VO2 max estimate was synced from Garmin in the last 30 days. |
| `lean_body_mass_kg` | float | — | Lean body mass in kg. Required for FFMI fitness scoring. |
| `height_cm` | float | — | Height in cm. When set, enables FFMI over flat-reference LBM. |
| `age` | int | — | Age in years. Reserved for future population norm adjustments. |

Example:

```toml
[user]
sleep_target_hours = 7.5
steps_target = 8000
lean_body_mass_kg = 72.0
height_cm = 178.0
age = 35
```

## `[providers.garmin]`

| Field | Type | Description |
|---|---|---|
| `enabled` | bool | Enable Garmin Connect sync |
| `username` | string | Garmin Connect email |
| `password` | string | Garmin Connect password |

```toml
[providers.garmin]
enabled = true
username = "you@example.com"
password = "your-password"
```

> **Note:** Credentials are stored in plaintext. Restrict file permissions: `chmod 600 ~/.pulse/config.toml`.

## `[providers.intervals]`

| Field | Type | Description |
|---|---|---|
| `enabled` | bool | Enable Intervals.icu sync |
| `athlete_id` | string | Your Intervals.icu athlete ID (from profile URL) |
| `api_key` | string | API key from account settings |

```toml
[providers.intervals]
enabled = true
athlete_id = "i12345"
api_key = "your-api-key"
```

## `[storage]`

| Field | Type | Default | Description |
|---|---|---|---|
| `raw_responses` | bool | `true` | Keep each provider response body in the database, so later versions can fill new columns and `pulse rebuild` can refill the tables without a re-sync. When off, a day synced again also loses the body stored for it earlier. |

```toml
[storage]
raw_responses = true
```

Roughly 25 to 90 MB a year, mostly Garmin sleep data.
