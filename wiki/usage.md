# Usage

## Commands

| Command | Flags | Description |
|---|---|---|
| `pulse config [init]` | — | Show current config, or `init` to create defaults |
| `pulse garmin-login` | `-u EMAIL`, `--mfa CODE` | Authenticate with Garmin Connect |
| `pulse sync [provider]` | `--days N`, `--from DATE`, `--to DATE` | Sync from all providers, or a specific one (`garmin`, `intervals`) |
| `pulse rebuild` | — | Refill the data tables from stored provider responses, without network access |
| `pulse vitality` | `--days N`, `--from DATE`, `--to DATE`, `--format FMT`, `--json` | Daily vitality score table |
| `pulse sleep` | `--days N`, `--from DATE`, `--to DATE`, `--format FMT`, `--json`, `--wide` | Sleep stages, score, HRV, bed and wake time |
| `pulse heart` | `--days N`, `--from DATE`, `--to DATE`, `--format FMT`, `--json`, `--wide` | Heart rate, HRV and VO2 max |
| `pulse recovery` | `--days N`, `--from DATE`, `--to DATE`, `--format FMT`, `--json`, `--wide` | Body Battery charged, drained, peak and low |
| `pulse activity` | `--days N`, `--from DATE`, `--to DATE`, `--format FMT`, `--json`, `--wide` | Steps, activity, calories and distance |
| `pulse stress` | `--days N`, `--from DATE`, `--to DATE`, `--format FMT`, `--json`, `--wide` | Stress scores and time at each stress level |
| `pulse workouts` | `--days N`, `--from DATE`, `--to DATE`, `--format FMT`, `--json`, `--wide` | Workout log with distance, pace, heart rate zones and training load |
| `pulse wellness` | `--days N`, `--from DATE`, `--to DATE`, `--format FMT`, `--json`, `--wide` | Weight, body fat, fitness (CTL) and fatigue (ATL) from Intervals.icu |

### Common flags

- `--days N` — the last N days, today included, so `--days 1` is today only (at least 1; default: 7; `sync`: 2, today and yesterday)
- `--from DATE`, `--to DATE` — a fixed range instead, as `YYYY-MM-DD`, both days included. `--to` defaults to today and needs `--from`; neither can be combined with `--days`. Every command refuses a `--to` after today.
- `--format <table|json|toon|ison>` — how to print the result (default: `table`)
- `--json` — shorthand for `--format json`; cannot be combined with `--format`
- `--wide` — `sleep`, `heart`, `recovery`, `activity`, `stress`, `workouts` and `wellness` only: the table shows every stored column, in the order of the JSON fields. For the table format only; it cannot be combined with `--json` or with a `--format` other than `table`. `workouts --wide` prints a table of workouts and a table of their exercise sets instead of the workout log.

> **Changed in 0.12.0:** `--days N` now returns exactly N dates. Before, it returned today and the N days before it, N+1 dates, so `--days 7` showed 8 days. Scripts that used `--days 1` for today and yesterday should use `--days 2`. The `sync` default moved from 1 to 2 to keep fetching today and yesterday. `--days 0` is no longer accepted; use `--days 1` for today only.

> **Changed in 0.14.0:** every command now refuses a `--to` after today, with `--to 2026-10-31 is after today (2026-10-18)`. Before, only `sync` did; the read commands accepted it, and `vitality` then counted its pace windows and baselines back from a day with no data. Scripts that pass a future `--to`, such as a month-end date in the middle of the month, now fail with that error. For "this month so far" use `--from 2026-10-01` alone, which ends today.

## Output formats

| Format | Use it for |
|---|---|
| `table` | Reading in a terminal |
| `json` | Scripts. Pretty-printed; the shape differs per command (an array of rows, an array of workouts with nested sets, or an object with `pace` and `scores` for `vitality`) |
| `toon` | Pasting into an LLM prompt. [TOON](https://github.com/toon-format/spec) declares the field names once per table instead of once per row |
| `ison` | The same use as `toon`, in [ISON](https://www.ison.dev/spec.html): one block per table, field names on one line, cells separated by spaces |

TOON output follows spec 4.4 and may change when the encoder is updated; use `--format json` where a stable interface matters. Every TOON document is a set of named tables, so the output of several commands can be concatenated into one prompt. `pulse sleep --format toon`:

```text
sleep[2]{date,total_seconds,deep_seconds,rem_seconds,light_seconds,awake_seconds,sleep_score,hrv_ms,start_time,end_time,spo2_avg,respiration_avg,source}:
  2026-01-02,27000,6300,5400,14400,900,82,48,"2026-01-01 23:10:00","2026-01-02 06:55:00",95,14.5,garmin
  2026-01-01,25200,5400,4800,14100,900,null,51.5,null,null,null,null,garmin
```

- `workouts` prints a `workouts` table, then the `hr_zone_seconds` and `hr_zone_max_bpm` tables (see below), then `exercise_sets` with one table of sets per workout, keyed by the workout id. The database row id of a set is left out. `exercise_sets: null` means no workout has sets. Its JSON nests the sets inside each workout instead.
- `workouts` carries `distance_meters` (meters) and `avg_speed_mps` (meters per second) as stored, in JSON too; only the terminal table converts them to km and pace per km. Both are `null` for a workout without distance, such as strength training.
- `workouts` carries `hr_zone_seconds`, the seconds spent in each heart rate zone, lowest zone first, as Intervals.icu counted them. JSON has it as an array on each workout. TOON and ISON print it as its own `hr_zone_seconds` table with one row per workout that has zone times: `workout_id`, then `z1` (lowest) to `zN`, with `null` for a zone that workout's zone setup did not have. `hr_zone_seconds: null` means no workout has zone times. A workout without heart rate time has `null` in JSON and no row in TOON and ISON. The terminal log prints `HR zones: Z1 8m · Z2 2m …` under the workout, leaving out zones with no time, and `<1m` means under a minute.
- `workouts` carries `hr_zone_max_bpm`, the top of each heart rate zone in bpm, lowest zone first: the limits Intervals.icu counted that workout's `hr_zone_seconds` against. JSON has it as an array on each workout. TOON and ISON print it as its own `hr_zone_max_bpm` table, shaped like `hr_zone_seconds`: one row per workout that has limits, `workout_id`, then `z1` (top of the lowest zone) to `zN`. `hr_zone_max_bpm: null` means no workout has limits. The terminal log puts each zone's top in front of its time, `HR zones: Z1 ≤123: 8m · Z2 ≤137: 2m …`, and prints the times without tops when a workout's number of limits differs from its number of zone times. `--wide` has a `Zone max bpm` column (`123/137/143/…`).
- `workouts` carries `training_load`, the workout's training load as Intervals.icu computed it, in whole points, in JSON, TOON and ISON alike; `null` when Intervals.icu computed none. The terminal log line ends with `load 43`, and `--wide` has a `Load` column.
- `wellness` carries `weight_kg` (kilograms), `body_fat_percent` (percent of body weight), `ctl` (fitness) and `atl` (fatigue), one row per day, in JSON, TOON and ISON alike. `weight_kg` and `body_fat_percent` are `null` on days without a measurement, including days on which Intervals.icu only copied an earlier weight. `ctl` and `atl` of `0` mean no training load yet, not a missing value.
- A set whose exercise Garmin did not identify has `null` for `exercise_name`, for `exercise_category`, or for both, in JSON, TOON and ISON. The key and the column are always present. The terminal table prints the name, else the category in parentheses, else `-`. A category in parentheses is the group Garmin filed the exercise under, which can be broader than the exercise itself: Upright Row is filed under `(Shrug)`. Releases up to 0.2.0 stored and printed `Unknown` for a missing value. 0.3.0 printed `-` even when the category was known.
- `vitality` prints a `pace` block followed by a `scores` table. `pace: null` means too few of the 7 or 30 days ending on the last day shown have a score (see [Pace of Aging](vitality-score.md#pace-of-aging)). Numbers are rounded as in the terminal table: two decimals for the pace multiplier, one for everything else. JSON keeps full precision.
- No data prints as empty tables (`sleep: []`).

> **Changed in 0.13.0:** the `vitality` pace compares the 7 days ending on the last day shown with the 30 days ending there, whatever range is shown, and needs a score on at least 5 of the 7 days and 20 of the 30. Before, both averages came from the days shown: with the default `--days 7` the multiplier was always `1.00`, and pace was `null` whenever fewer than 7 days had a score. The fields keep their names; their values can differ from earlier releases for the same dates.

TOON and ISON are compact views, not lossless copies of the JSON output.

ISON output follows [spec v1.0](https://www.ison.dev/spec.html) and shows the same compact view as TOON: the same tables, the same rounding, no set row ids. `pulse sleep --format ison`:

```text
table.sleep
date total_seconds deep_seconds rem_seconds light_seconds awake_seconds sleep_score hrv_ms start_time end_time spo2_avg respiration_avg source
2026-01-02 27000 6300 5400 14400 900 82 48.0 "2026-01-01 23:10:00" "2026-01-02 06:55:00" 95.0 14.5 garmin
2026-01-01 25200 5400 4800 14100 900 null 51.5 null null null null garmin
```

- `start_time` and `end_time` of `sleep` are local wall-clock times, `YYYY-MM-DD HH:MM:SS` with no offset; TOON and ISON quote them.

- Each table is a block: `table.<name>`, a line of field names, then one line per row. Blocks are separated by an empty line.
- A value is quoted when it holds a space, a quote or a backslash, or would otherwise read as a number, `true`, `false`, `null`, a reference or a comment: `"Bench Press"`, `"21234567890"`.
- A decimal number keeps its decimal point (`48.0`), because ISON reads `48` as an integer.
- `workouts` prints `table.workouts`, then `table.hr_zone_seconds` and `table.hr_zone_max_bpm` when any workout has them, then one `table.exercise_sets.<workout id>` block for each workout that has sets.
- `vitality` prints an `object.pace` block, then `table.scores`. There is no `object.pace` block when there is no pace (`pace: null` in JSON and TOON).
- A table with no rows is left out; no data at all prints an empty document.
- To put the output of several commands into one prompt, separate them with an empty line.

## Vitality Score

The vitality score is a daily 0–100 composite. Each component is scored independently against targets or population norms, then combined with the following weights:

| Component | Weight | Source |
|---|---|---|
| Sleep | 25% | Duration vs `sleep_target_hours`, Garmin sleep score |
| Recovery | 20% | Body Battery peak, HRV and resting heart rate against their baselines |
| Activity | 20% | Steps vs `steps_target` |
| Stress | 15% | Garmin stress score (lower = better) |
| Fitness | 20% | VO2 max percentile + FFMI (if LBM configured) |

Fitness counts only when a VO2 max value or `lean_body_mass_kg` is available; without either, the other four are weighted 30/25/25/20. Fitness scoring uses **Shvartz & Reinbold** population norms for VO2 max percentiles and FFMI calculated from `lean_body_mass_kg` and `height_cm`.

See [Vitality score](vitality-score.md) for the formulas, the pace trend and a worked example.

## Examples

```bash
# September's vitality, scored against September's baselines
pulse vitality --from 2026-09-01 --to 2026-09-30

# Sleep since 1 October
pulse sleep --from 2026-10-01

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

# Every stored sleep column, as a table
pulse sleep --wide
```
