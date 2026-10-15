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
- `--format <table|json|toon|ison>` — how to print the result (default: `table`)
- `--json` — shorthand for `--format json`; cannot be combined with `--format`

## Output formats

| Format | Use it for |
|---|---|
| `table` | Reading in a terminal |
| `json` | Scripts. Pretty-printed; the shape differs per command (an array of rows, an array of workouts with nested sets, or an object with `pace` and `scores` for `vitality`) |
| `toon` | Pasting into an LLM prompt. [TOON](https://github.com/toon-format/spec) declares the field names once per table instead of once per row |
| `ison` | The same use as `toon`, in [ISON](https://www.ison.dev/spec.html): one block per table, field names on one line, cells separated by spaces |

TOON output follows spec 4.4 and may change when the encoder is updated; use `--format json` where a stable interface matters. Every TOON document is a set of named tables, so the output of several commands can be concatenated into one prompt. `pulse sleep --format toon`:

```text
sleep[2]{date,total_seconds,deep_seconds,rem_seconds,light_seconds,awake_seconds,sleep_score,hrv_ms,source}:
  2026-01-02,27000,6300,5400,14400,900,82,48,garmin
  2026-01-01,25200,5400,4800,14100,900,null,51.5,garmin
```

- `workouts` prints a `workouts` table, then `exercise_sets` with one table of sets per workout, keyed by the workout id. The database row id of a set is left out. `exercise_sets: null` means no workout has sets. Its JSON nests the sets inside each workout instead.
- A set whose exercise Garmin did not identify has `null` for `exercise_name`, for `exercise_category`, or for both, in JSON, TOON and ISON. The key and the column are always present. The terminal table prints the name, else the category in parentheses, else `-`. A category in parentheses is the group Garmin filed the exercise under, which can be broader than the exercise itself: Upright Row is filed under `(Shrug)`. Releases up to 0.2.0 stored and printed `Unknown` for a missing value. 0.3.0 printed `-` even when the category was known.
- `vitality` prints a `pace` block (`pace: null` until seven days are scored) followed by a `scores` table. Numbers are rounded as in the terminal table: two decimals for the pace multiplier, one for everything else. JSON keeps full precision.
- No data prints as empty tables (`sleep: []`).

TOON and ISON are compact views, not lossless copies of the JSON output.

ISON output follows [spec v1.0](https://www.ison.dev/spec.html) and shows the same compact view as TOON: the same tables, the same rounding, no set row ids. `pulse sleep --format ison`:

```text
table.sleep
date total_seconds deep_seconds rem_seconds light_seconds awake_seconds sleep_score hrv_ms source
2026-01-02 27000 6300 5400 14400 900 82 48.0 garmin
2026-01-01 25200 5400 4800 14100 900 null 51.5 garmin
```

- Each table is a block: `table.<name>`, a line of field names, then one line per row. Blocks are separated by an empty line.
- A value is quoted when it holds a space, a quote or a backslash, or would otherwise read as a number, `true`, `false`, `null`, a reference or a comment: `"Bench Press"`, `"21234567890"`.
- A decimal number keeps its decimal point (`48.0`), because ISON reads `48` as an integer.
- `workouts` prints `table.workouts`, then one `table.exercise_sets.<workout id>` block for each workout that has sets.
- `vitality` prints an `object.pace` block, then `table.scores`. There is no `object.pace` block until seven days are scored.
- A table with no rows is left out; no data at all prints an empty document.
- To put the output of several commands into one prompt, separate them with an empty line.

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

# Last 30 days of workouts as ISON, for an LLM prompt
pulse workouts --days 30 --format ison

# Sync only Garmin for the past 2 days
pulse sync garmin --days 2

# Show sleep table for the past week (default)
pulse sleep
```
