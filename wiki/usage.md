# Usage

## Commands

| Command | Flags | Description |
|---|---|---|
| `pulse config [init]` | — | Show current config, or `init` to create defaults |
| `pulse garmin-login` | `-u EMAIL`, `--mfa CODE` | Authenticate with Garmin Connect |
| `pulse sync [provider]` | `--days N` | Sync from all providers, or a specific one (`garmin`, `intervals`) |
| `pulse vitality` | `--days N`, `--format FMT`, `--json` | Daily vitality score table |
| `pulse sleep` | `--days N`, `--format FMT`, `--json` | Sleep data table |
| `pulse heart` | `--days N`, `--format FMT`, `--json` | Heart rate data |
| `pulse recovery` | `--days N`, `--format FMT`, `--json` | HRV and recovery scores |
| `pulse activity` | `--days N`, `--format FMT`, `--json` | Steps and activity |
| `pulse stress` | `--days N`, `--format FMT`, `--json` | Stress scores |
| `pulse workouts` | `--days N`, `--format FMT`, `--json` | Workout log |

### Common flags

- `--days N` — number of past days to show (default: 7)
- `--format <table|json|toon>` — how to print the result (default: `table`)
- `--json` — shorthand for `--format json`; cannot be combined with `--format`

## Output formats

| Format | Use it for |
|---|---|
| `table` | Reading in a terminal |
| `json` | Scripts. Pretty-printed; the shape differs per command (an array of rows, an array of workouts with nested sets, or an object with `pace` and `scores` for `vitality`) |
| `toon` | Pasting into an LLM prompt. [TOON](https://github.com/toon-format/spec) declares the field names once per table instead of once per row |

TOON output follows spec 4.4 and may change when the encoder is updated; use `--format json` where a stable interface matters. Every TOON document is a set of named tables, so the output of several commands can be concatenated into one prompt. `pulse sleep --format toon`:

```text
sleep[2]{date,total_seconds,deep_seconds,rem_seconds,light_seconds,awake_seconds,sleep_score,hrv_ms,source}:
  2026-01-02,27000,6300,5400,14400,900,82,48,garmin
  2026-01-01,25200,5400,4800,14100,900,null,51.5,garmin
```

- `workouts` prints two tables, `workouts` and `exercise_sets`, joined on `workout_id`. Its JSON nests the sets inside each workout instead.
- `vitality` prints a `pace` block (`pace: null` until seven days are scored) followed by a `scores` table.
- No data prints as empty tables (`sleep: []`).

## Vitality Score

The vitality score is a daily 0–100 composite. Each component is scored independently against targets or population norms, then combined with the following weights:

| Component | Weight | Source |
|---|---|---|
| Sleep | 25% | Duration vs `sleep_target_hours`, quality metrics |
| Recovery | 20% | HRV score from Garmin |
| Activity | 20% | Steps vs `steps_target` |
| Stress | 15% | Garmin stress score (lower = better) |
| Fitness | 20% | VO2 max percentile + FFMI (if LBM configured) |

Fitness scoring uses **Shvartz & Reinbold** population norms for VO2 max percentiles and FFMI calculated from `lean_body_mass_kg` and `height_cm`.

## Examples

```bash
# Last 30 days of vitality scores as JSON
pulse vitality --days 30 --json

# Last 30 days of sleep as TOON, for an LLM prompt
pulse sleep --days 30 --format toon

# Sync only Garmin for the past 2 days
pulse sync garmin --days 2

# Show sleep table for the past week (default)
pulse sleep
```
