# Vitality Score

**Location:** `~/.openclaw/bin/vitality`  
**Config:** `~/.openclaw/vitality-config.json`  
**Created:** 2026-02-17  
**Purpose:** WHOOP-inspired "Pace of Aging" metric from Garmin data

---

## Overview

Calculates a composite health score (0-100) comparing your recent performance (7-day average) against your baseline (30-day average) to determine your "pace of aging."

**Inspired by:** WHOOP's Strain/Recovery scores  
**Data source:** Garmin Connect (via `python-garminconnect`)

---

## Score Components

| Component | Weight | Data Sources |
|-----------|--------|--------------|
| **Sleep** | 30% | Duration, consistency, deep sleep % |
| **Recovery** | 25% | Resting HR, HRV, Body Battery |
| **Activity** | 25% | Steps, active minutes, exercise |
| **Stress** | 20% | Average daily stress level |

**Final score:** Weighted average (0-100)

---

## Pace of Aging Interpretation

Compares 7-day average vs 30-day baseline:

| Pace | Multiplier | Meaning |
|------|------------|---------|
| 🟢 0.5x - 0.8x | Slowing | Exceptional recovery, body rebuilding |
| 🟡 0.8x - 1.2x | Steady | Normal maintenance mode |
| 🔴 1.2x - 2.0x+ | Accelerating | Depleting reserves, need rest |

**Example:** Score 58.4 with pace 0.83x = "steady" — maintaining well

---

## Usage

```bash
# Today's score with 7-day trend
vitality

# Output:
# 58.4/100 — Pace: 0.83x 🟡 steady (+1.5 vs 30d baseline)

# Full 30-day history
vitality --days 30

# Machine-readable JSON
vitality --json

# Pre-formatted for Obsidian debrief
vitality --debrief

# Update config value
vitality --set age 38
vitality --set sleep_target_hours 8.0
```

---

## Configuration

**Path:** `~/.openclaw/vitality-config.json`

```json
{
  "age": 37,
  "lean_body_mass_kg": null,
  "vo2_max": null,
  "sleep_target_hours": 7.5,
  "steps_target": 8000,
  "rhr_baseline": null,
  "hrv_baseline": null
}
```

| Field | Purpose | Auto-calculated? |
|-------|---------|------------------|
| `age` | Age in years (affects targets) | ❌ Manual |
| `sleep_target_hours` | Personal sleep goal | ❌ Manual |
| `steps_target` | Daily step goal | ❌ Manual |
| `rhr_baseline` | Your typical resting HR | ✅ Auto (30d avg) |
| `hrv_baseline` | Your typical HRV | ✅ Auto (30d avg) |
| `vo2_max` | Fitness level if known | ❌ Optional |

**Auto-calculated baselines** update weekly from your Garmin data.

---

## Installation

### Prerequisites

```bash
# Garmin venv must exist with python-garminconnect
~/.openclaw/venvs/garmin/bin/pip install garminconnect
```

### Deploy Script

```bash
cp /path/to/vitality ~/.openclaw/bin/
chmod +x ~/.openclaw/bin/vitality
```

### Create Config

```bash
cat > ~/.openclaw/vitality-config.json << 'EOF'
{
  "age": 37,
  "sleep_target_hours": 7.5,
  "steps_target": 8000
}
EOF
```

### Garmin Auth

```bash
garmin auth login
# Follow prompts to authenticate
# Tokens saved to ~/.clawdbot/garmin/
```

---

## Data Flow

```
Garmin Connect API
        ↓
python-garminconnect (venv)
        ↓
Fetch: sleep, hr, hrv, body_battery, steps, stress
        ↓
Calculate 7-day avg vs 30-day baseline
        ↓
Weighted score (0-100)
        ↓
Pace multiplier + trend arrow
        ↓
Output: Terminal / JSON / Debrief format
```

---

## Cron Integration

**Morning Briefing** (08:00):
```bash
# Part of briefing payload
vitality --debrief  # → includes in Obsidian journal
```

**Nightly Debrief** (22:00):
```bash
# Part of debrief payload  
vitality --days 30  # → trend analysis
```

---

## Output Formats

### Default (Human)
```
58.4/100 — Pace: 0.83x 🟡 steady (+1.5 vs 30d baseline)

Components:
  😴 Sleep: 60.1 | 💚 Recovery: 53.9
  🏃 Activity: 50.4 | 🧘 Stress: 71.7
```

### JSON (--json)
```json
{
  "score": 58.4,
  "pace": 0.83,
  "trend": "steady",
  "vs_baseline": 1.5,
  "components": {
    "sleep": 60.1,
    "recovery": 53.9,
    "activity": 50.4,
    "stress": 71.7
  }
}
```

### Debrief (--debrief)
```markdown
### 🔋 Vitality Score
**58.4/100** — Pace: 0.83x 🟡 steady (+1.5 vs baseline)

| Component | Score | Detail |
|-----------|-------|--------|
| 😴 Sleep | 60.1 | 5.1h avg |
| 💚 Recovery | 53.9 | HRV 42ms, RHR 54.9bpm |
| 🏃 Activity | 50.4 | 6,339 steps, 12.4 min active |
| 🧘 Stress | 71.7 | avg 34.1 |
```

---

## Troubleshooting

| Issue | Cause | Fix |
|-------|-------|-----|
| "Garmin auth failed" | Expired tokens | `garmin auth login` |
| "No data" | Sync delay | Wait 5-10 min post-workout |
| Score jumps wildly | Missing baseline | Wait 30 days of data |
| Pace always 1.0x | Flat trends | Normal — means stability |

---

## Related

- **Skill:** `garmin-health-analysis` (provides data foundation)
- **Script:** `~/.openclaw/bin/garmin` (auth wrapper)
- **Venv:** `~/.openclaw/venvs/garmin/`
- **Daily System:** See [daily-system.md](daily-system.md)

---

**Last Updated:** 2026-02-28
