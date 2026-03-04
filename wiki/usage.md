# Usage

## Commands

| Command | Flags | Description |
|---|---|---|
| `pulse config [init]` | — | Show current config, or `init` to create defaults |
| `pulse garmin-login` | `-u EMAIL`, `--mfa CODE` | Authenticate with Garmin Connect |
| `pulse sync [provider]` | `--days N` | Sync from all providers, or a specific one (`garmin`, `intervals`) |
| `pulse vitality` | `--days N`, `--json` | Daily vitality score table |
| `pulse sleep` | `--days N`, `--json` | Sleep data table |
| `pulse heart` | `--days N`, `--json` | Heart rate data |
| `pulse recovery` | `--days N`, `--json` | HRV and recovery scores |
| `pulse activity` | `--days N`, `--json` | Steps and activity |
| `pulse stress` | `--days N`, `--json` | Stress scores |
| `pulse workouts` | `--days N`, `--json` | Workout log |

### Common flags

- `--days N` — number of past days to show (default: 7)
- `--json` — output as JSON array instead of a table

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

# Sync only Garmin for the past 2 days
pulse sync garmin --days 2

# Show sleep table for the past week (default)
pulse sleep
```
