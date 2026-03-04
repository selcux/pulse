# Vitality Score

The vitality score is a daily **0–100 composite** that summarises how well you slept, recovered, moved, and managed stress. When fitness data is available it is included as an additional component.

Sleep records act as the anchor — a score is produced for every date that has sleep data. If a component has no data for a given day it falls back to a neutral **50**.

---

## Component Weights

| Component | Without fitness | With fitness | Source data |
|---|---|---|---|
| Sleep | 30% | 25% | Duration vs `sleep_target_hours` + device sleep score |
| Recovery | 25% | 20% | Body battery peak, HRV, RHR |
| Activity | 25% | 20% | Steps vs `steps_target` |
| Stress | 20% | 15% | Garmin average stress (inverted) |
| Fitness | — | 20% | VO2 max percentile + FFMI |

Fitness is **optional**: it is included only when VO2 max data is available (synced from Garmin or set via `vo2_max` in config). When it is absent the other four weights re-normalise to sum to 100%.

---

## Component Formulas

### Sleep (0–100)

```
duration_score = clamp(actual_seconds / target_seconds × 100, 0, 100)
```

If the device also provides a sleep quality score (Garmin sleep score):

```
sleep_score = (duration_score + device_sleep_score) / 2
```

Configure the target with `sleep_target_hours` (default: 8 h).

---

### Recovery (0–100)

Average of whichever of these are available for the day:

| Sub-component | Formula |
|---|---|
| Body battery peak | Raw value (already 0–100) |
| HRV | `clamp(hrv / hrv_baseline × 100, 0, 100)` |
| RHR | `clamp((2 − rhr / rhr_baseline) × 100, 0, 100)` |

**Baselines** are resolved in priority order:
1. `rhr_baseline` / `hrv_baseline` from config — explicit personal override
2. 30-day rolling average computed from synced heart data
3. Hardcoded fallbacks: RHR = 60 bpm, HRV = 50 ms

At baseline, both HRV and RHR contribute 100. A RHR 10% above baseline scores ~90; a RHR 10% below baseline scores 110 (capped at 100).

---

### Activity (0–100)

```
activity_score = clamp(steps / steps_target × 100, 0, 100)
```

Configure the target with `steps_target` (default: 10,000). Exceeding the target is capped at 100.

---

### Stress (0–100)

Garmin stress is inverted — lower stress yields a higher score.

```
stress_score = clamp(100 − avg_stress, 0, 100)
```

Garmin's range is 0–100. A typical resting day around 25 produces a stress score of 75.

---

### Fitness (0–100, optional)

The fitness score is the average of whichever sub-components are available:

#### VO2 Max percentile

Scored against **Shvartz & Reinbold (1990)** population norms using piecewise linear interpolation across the Finnish fitness classification bands:

| VO2 max (ml/kg/min) | Band | Score |
|---|---|---|
| < 26 | Very poor | 0–14.3 |
| 26–32 | Poor | 14.3–28.6 |
| 32–36 | Fair | 28.6–42.9 |
| 36–42 | Average | 42.9–57.1 |
| 42–47 | Good | 57.1–71.4 |
| 47–52 | Very good | 71.4–85.7 |
| 52–60 | Excellent | 85.7–100 |
| > 60 | Elite | 100 (capped) |

VO2 max source priority:
1. Value synced from Garmin for that specific day
2. 30-day average of synced Garmin values
3. Manual `vo2_max` in config

#### FFMI (Fat-Free Mass Index)

Requires `lean_body_mass_kg` in config.

```
ffmi = lbm_kg / height_m²
ffmi_score = clamp(ffmi / 25 × 100, 0, 100)
```

The ceiling of 25 represents the approximate natural athlete maximum (no pharmacological enhancement). At FFMI 25 the score is 100.

If `height_cm` is not set, a flat 70 kg reference is used instead:

```
lbm_score = clamp(lbm_kg / 70 × 100, 0, 100)
```

---

## Pace of Aging

The `vitality` command also computes a **pace** metric when at least 7 days of scores are available:

| Field | Meaning |
|---|---|
| `seven_day_avg` | Mean score over the last 7 days |
| `thirty_day_avg` | Mean score over the full window requested |
| `multiplier` | `seven_day_avg / thirty_day_avg` |
| `vs_baseline` | `seven_day_avg − thirty_day_avg` (signed) |
| `trend` | `Slowing` (≥ 1.1), `Steady` (0.9–1.1), or `Accelerating` (< 0.9) |

> **Terminology note:** "Slowing" means your recent scores are *higher* than your baseline — you are aging more slowly. "Accelerating" means recent scores are lower — a signal to investigate.

---

## Worked Example

Given these readings on a single day:

| Metric | Value |
|---|---|
| Sleep | 7 h of 8 h target, device score 80 |
| Body battery peak | 80 |
| HRV | 45 ms (baseline 50 ms) |
| RHR | 60 bpm (baseline 60 bpm) |
| Steps | 8,000 of 10,000 target |
| Avg stress | 30 |
| VO2 max | 42 ml/kg/min |

Component scores:

```
sleep    = (7/8×100 + 80) / 2  = (87.5 + 80) / 2 = 83.75
recovery = avg(80, 90, 100)    = 90.0      # battery=80, hrv=45/50×100=90, rhr=100
activity = 8000/10000×100      = 80.0
stress   = 100 - 30            = 70.0
fitness  = vo2_population_score(42) ≈ 57.1
```

Total (with fitness):

```
83.75×0.25 + 90.0×0.20 + 80.0×0.20 + 70.0×0.15 + 57.1×0.20
= 20.94 + 18.00 + 16.00 + 10.50 + 11.42
≈ 76.9
```
